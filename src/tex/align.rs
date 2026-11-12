//! Alignments: `\halign` and `\valign` (§768–§812).

use std::sync::Arc;

use rtex_core::cmd::{self, chr};
use rtex_core::{CsId, Glue, GlueOrder, GroupKind, Loc, Res, Token, Value};

use super::build::{normal_paragraph, scan_spec};
use super::node::{BoxDir, BoxNode, GlueKind, GlueNode, GlueSign, Node, RUNNING, UnsetNode};
use super::pack::{NATURAL, append_to_vlist, hpack, param_glue, vpack, vpackage};
use super::{Eng, HMODE, IGNORE_DEPTH, MMODE, Spec, VMODE, pop_nest, push_nest, tail_append};
use crate::units::Scaled;

const TAB_SKIP: usize = 11;
const EVERY_CR: usize = 7;
const GLOBAL_DEFS: usize = 43;
const OVERFULL_RULE: usize = 16;
const DISPLAY_INDENT: usize = 15;
const ALIGN_LOW: i32 = -1_000_000;
const ALIGN_HIGH: i32 = 1_000_000;

#[derive(Clone, Debug)]
struct Record {
    u_part: Arc<[Token]>,
    v_part: Arc<[Token]>,
    width: Scaled,
    spans: Vec<(usize, Scaled)>,
    extra: u32,
}

/// One level of the alignment stack (§770).
#[derive(Clone, Debug)]
pub(crate) struct Alignment {
    glues: Vec<Node>,
    records: Vec<Record>,
    cur_align: Option<usize>,
    cur_span: usize,
    cur_loop: Option<usize>,
    saved_align_state: i32,
    adjust: Vec<Node>,
    spec: Spec,
}

fn tabskip_node(spec: Glue, zero: bool) -> Node {
    Node::Glue(GlueNode {
        spec,
        kind: GlueKind::Param(TAB_SKIP as u8),
        leader: None,
        zero_ptr: zero,
    })
}

fn glue_spec(n: &Node) -> (Glue, bool) {
    match n {
        Node::Glue(g) => (g.spec, g.zero_ptr),
        _ => (Glue::default(), true),
    }
}

/// `get_preamble_token` (§782).
fn get_preamble_token(eng: &mut Eng) -> Res {
    loop {
        eng.get_token()?;
        while eng.cur_chr() == chr::SPAN && eng.cur_cmd() == cmd::TAB_MARK {
            eng.get_token()?;
            if eng.cur_cmd() > cmd::MAX_COMMAND {
                eng.expand()?;
                eng.get_token()?;
            }
        }
        if eng.cur_cmd() == cmd::ENDV {
            return eng.fatal_error("(interwoven alignment preambles are not allowed)");
        }
        if eng.cur_cmd() == cmd::ASSIGN_GLUE && eng.cur_chr() == TAB_SKIP as u32 {
            eng.scan_optional_equals()?;
            let g = eng.scan_glue(chr::GLUE_VAL)?;
            let global = eng.int_par(GLOBAL_DEFS) > 0;
            eng.eq_define(Loc::GluePar(TAB_SKIP as u16), Value::Glue(g), global)?;
            continue;
        }
        return Ok(());
    }
}

fn current(eng: &mut Eng) -> &mut Alignment {
    eng.stomach.aligns.last_mut().expect("an alignment in progress")
}

/// `init_align` (§774).
pub(crate) fn init_align(eng: &mut Eng) -> Res {
    let save_cs: Option<CsId> = eng.cur_cs();
    let outer_align_state = *eng.align_state_mut();
    eng.stomach.aligns.push(Alignment {
        glues: Vec::new(),
        records: Vec::new(),
        cur_align: None,
        cur_span: 0,
        cur_loop: None,
        saved_align_state: outer_align_state,
        adjust: Vec::new(),
        spec: NATURAL,
    });
    *eng.align_state_mut() = ALIGN_LOW;
    if eng.mode == MMODE && super::math::display_has_material(eng) {
        eng.print_err("Improper ");
        eng.print_esc("halign");
        eng.print_str(" inside $$'s");
        eng.set_help(&[
            "Displays can use special alignments (like \\eqalignno)",
            "only if nothing but the alignment itself is between $$'s.",
            "So I've deleted the formulas that preceded this alignment.",
        ]);
        eng.error()?;
        super::math::flush_math(eng);
    }
    push_nest(eng);
    if eng.mode == MMODE {
        eng.mode = -VMODE;
        let n = eng.stomach.nest.len();
        if n >= 2 {
            eng.aux.prev_depth = eng.stomach.nest[n - 2].prev_depth;
        }
    } else if eng.mode > 0 {
        eng.mode = -eng.mode;
    }
    let spec = scan_spec(eng)?;
    current(eng).spec = spec;
    eng.new_save_level(GroupKind::Align)?;
    eng.scan_left_brace()?;
    eng.set_aligning(true);
    eng.set_warning_index(save_cs);
    *eng.align_state_mut() = ALIGN_LOW;
    loop {
        let g = param_glue(eng, TAB_SKIP);
        current(eng).glues.push(g);
        if eng.cur_cmd() == cmd::CAR_RET {
            break;
        }
        eng.stomach.preamble_tokens.clear();
        loop {
            get_preamble_token(eng)?;
            if eng.cur_cmd() == cmd::MAC_PARAM {
                break;
            }
            let delim = (cmd::TAB_MARK..=cmd::CAR_RET).contains(&eng.cur_cmd()) && *eng.align_state_mut() == ALIGN_LOW;
            if delim {
                let empty = eng.stomach.preamble_tokens.is_empty();
                let al = current(eng);
                if empty && al.cur_loop.is_none() && eng.cur_cmd() == cmd::TAB_MARK {
                    let g = current(eng).glues.len() - 1;
                    current(eng).cur_loop = Some(g);
                } else {
                    eng.print_err("Missing # inserted in alignment preamble");
                    eng.set_help(&[
                        "There should be exactly one # between &'s, when an",
                        "\\halign or \\valign is being set up. In this case you had",
                        "none, so I've put one in; maybe that will work.",
                    ]);
                    eng.back_error()?;
                    break;
                }
            } else if eng.cur_cmd() != cmd::SPACER || !eng.stomach.preamble_tokens.is_empty() {
                let t = eng.cur_tok();
                eng.stomach.preamble_tokens.push(t);
            }
        }
        let u = std::mem::take(&mut eng.stomach.preamble_tokens);
        loop {
            get_preamble_token(eng)?;
            if (cmd::TAB_MARK..=cmd::CAR_RET).contains(&eng.cur_cmd()) && *eng.align_state_mut() == ALIGN_LOW {
                break;
            }
            if eng.cur_cmd() == cmd::MAC_PARAM {
                eng.print_err("Only one # is allowed per tab");
                eng.set_help(&[
                    "There should be exactly one # between &'s, when an",
                    "\\halign or \\valign is being set up. In this case you had",
                    "more than one, so I'm ignoring all but the first.",
                ]);
                eng.error()?;
                continue;
            }
            let t = eng.cur_tok();
            eng.stomach.preamble_tokens.push(t);
        }
        let mut v = std::mem::take(&mut eng.stomach.preamble_tokens);
        v.push(eng.end_template_token());
        current(eng).records.push(Record {
            u_part: Arc::from(u),
            v_part: Arc::from(v),
            width: RUNNING,
            spans: Vec::new(),
            extra: 0,
        });
    }
    eng.set_aligning(false);
    eng.new_save_level(GroupKind::Align)?;
    eng.begin_toks_param(EVERY_CR)?;
    align_peek(eng)
}

/// `align_peek` (§785).
fn align_peek(eng: &mut Eng) -> Res {
    loop {
        *eng.align_state_mut() = ALIGN_HIGH;
        eng.get_next_nonblank_noncall()?;
        if eng.cur_cmd() == cmd::NO_ALIGN {
            eng.scan_left_brace()?;
            eng.new_save_level(GroupKind::NoAlign)?;
            if eng.mode == -VMODE {
                normal_paragraph(eng)?;
            }
            return Ok(());
        }
        if eng.cur_cmd() == cmd::RIGHT_BRACE {
            return fin_align(eng);
        }
        if eng.cur_cmd() == cmd::CAR_RET && eng.cur_chr() == chr::CR_CR {
            continue;
        }
        init_row(eng);
        return init_col(eng);
    }
}

/// `init_row` (§786).
fn init_row(eng: &mut Eng) {
    push_nest(eng);
    eng.mode = (-HMODE - VMODE) - eng.mode;
    if eng.mode == -HMODE {
        eng.aux.space_factor = 0;
    } else {
        eng.aux.prev_depth = 0;
    }
    let (spec, zero) = glue_spec(&current(eng).glues[0]);
    tail_append(eng, tabskip_node(spec, zero));
    let al = current(eng);
    al.cur_align = Some(0);
    al.adjust.clear();
    init_span(eng, 0);
}

/// `init_span` (§787).
fn init_span(eng: &mut Eng, p: usize) {
    push_nest(eng);
    if eng.mode == -HMODE {
        eng.aux.space_factor = 1000;
    } else {
        eng.aux.prev_depth = IGNORE_DEPTH;
        let _ = normal_paragraph(eng);
    }
    current(eng).cur_span = p;
}

/// `init_col` (§788).
fn init_col(eng: &mut Eng) -> Res {
    let c = u32::from(eng.cur_cmd());
    let al = current(eng);
    let idx = al.cur_align.expect("a current column");
    al.records[idx].extra = c;
    if c == u32::from(cmd::OMIT) {
        *eng.align_state_mut() = 0;
        Ok(())
    } else {
        let u = current(eng).records[idx].u_part.clone();
        eng.back_input()?;
        eng.begin_template(u, false)
    }
}

/// Inserts the `v` part when an entry ends (§789).
pub(crate) fn insert_v_part(eng: &mut Eng) -> Res {
    let chr = eng.cur_chr();
    let Some(al) = eng.stomach.aligns.last_mut() else {
        return eng.fatal_error("(interwoven alignment preambles are not allowed)");
    };
    let Some(idx) = al.cur_align else {
        return eng.fatal_error("(interwoven alignment preambles are not allowed)");
    };
    let saved = std::mem::replace(&mut al.records[idx].extra, chr);
    let toks = if saved == u32::from(cmd::OMIT) {
        Arc::from(vec![eng.end_template_token()])
    } else {
        al.records[idx].v_part.clone()
    };
    eng.begin_template(toks, true)?;
    *eng.align_state_mut() = ALIGN_HIGH;
    Ok(())
}

/// `do_endv` (§1131).
pub(crate) fn do_endv(eng: &mut Eng) -> Res {
    if !eng.v_template_ended() {
        return eng.fatal_error("(interwoven alignment preambles are not allowed)");
    }
    if eng.cur_group() == GroupKind::Align {
        super::build::end_graf(eng)?;
        if fin_col(eng)? {
            fin_row(eng)?;
        }
        Ok(())
    } else {
        eng.off_save()
    }
}

/// `fin_col` (§791): whether a row has ended too.
fn fin_col(eng: &mut Eng) -> Res<bool> {
    let Some(cur) = eng.stomach.aligns.last().and_then(|a| a.cur_align) else {
        eng.confusion("endv")?;
        return Ok(false);
    };
    if *eng.align_state_mut() < 500_000 {
        eng.fatal_error("(interwoven alignment preambles are not allowed)")?;
    }
    let mut p = if cur + 1 < current(eng).records.len() {
        Some(cur + 1)
    } else {
        None
    };
    if p.is_none() && current(eng).records[cur].extra < chr::CR {
        if let Some(lp) = current(eng).cur_loop {
            let al = current(eng);
            let src = al.records[lp].clone();
            al.records.push(Record {
                u_part: src.u_part,
                v_part: src.v_part,
                width: RUNNING,
                spans: Vec::new(),
                extra: 0,
            });
            let next_loop = lp + 1;
            let glue = al.glues[next_loop].clone();
            al.glues.push(glue);
            al.cur_loop = Some(next_loop);
            p = Some(al.records.len() - 1);
        } else {
            eng.print_err("Extra alignment tab has been changed to ");
            eng.print_esc("cr");
            eng.set_help(&[
                "You have given more \\span or & marks than there were",
                "in the preamble to the \\halign or \\valign now in progress.",
                "So I'll assume that you meant to type \\cr instead.",
            ]);
            current(eng).records[cur].extra = chr::CR;
            eng.error()?;
        }
    }
    let extra = current(eng).records[cur].extra;
    if extra != chr::SPAN {
        eng.unsave()?;
        eng.new_save_level(GroupKind::Align)?;
        let list = std::mem::take(&mut eng.stomach.cur);
        let (u, w) = if eng.mode == -HMODE {
            eng.stomach.adjust = Some(std::mem::take(&mut current(eng).adjust));
            let u = hpack(eng, list, NATURAL);
            current(eng).adjust = eng.stomach.adjust.take().unwrap_or_default();
            let w = u.width;
            (u, w)
        } else {
            let u = vpackage(eng, list, NATURAL, 0);
            let w = u.height;
            (u, w)
        };
        let (stretch, shrink) = eng.stomach.pack_totals;
        let al = current(eng);
        let span = al.cur_span;
        let n = cur - span;
        if n > 0 {
            let spans = &mut al.records[span].spans;
            match spans.iter_mut().find(|(m, _)| *m == n) {
                Some((_, sw)) => *sw = (*sw).max(w),
                None => {
                    spans.push((n, w));
                    spans.sort_by_key(|(m, _)| *m);
                }
            }
        } else if w > al.records[cur].width {
            al.records[cur].width = w;
        }
        let order = |a: &[Scaled; 4]| {
            if a[3] != 0 {
                GlueOrder::Filll
            } else if a[2] != 0 {
                GlueOrder::Fill
            } else if a[1] != 0 {
                GlueOrder::Fil
            } else {
                GlueOrder::Normal
            }
        };
        let so = order(&stretch);
        let ho = order(&shrink);
        let unset = UnsetNode {
            width: u.width,
            depth: u.depth,
            height: u.height,
            span_count: n as u16,
            stretch: stretch[so as usize],
            shrink: shrink[ho as usize],
            stretch_order: so,
            shrink_order: ho,
            list: u.list,
        };
        pop_nest(eng);
        tail_append(eng, Node::Unset(Box::new(unset)));
        let (spec, zero) = glue_spec(&current(eng).glues[cur + 1]);
        tail_append(eng, tabskip_node(spec, zero));
        if extra >= chr::CR {
            return Ok(true);
        }
        if let Some(p) = p {
            init_span(eng, p);
        }
    }
    *eng.align_state_mut() = ALIGN_HIGH;
    eng.get_next_nonblank_noncall()?;
    current(eng).cur_align = p;
    init_col(eng)?;
    Ok(false)
}

fn unset_from(b: BoxNode) -> Node {
    Node::Unset(Box::new(UnsetNode {
        width: b.width,
        depth: b.depth,
        height: b.height,
        span_count: 0,
        stretch: 0,
        shrink: 0,
        stretch_order: b.glue_order,
        shrink_order: GlueOrder::Normal,
        list: b.list,
    }))
}

/// `fin_row` (§799).
fn fin_row(eng: &mut Eng) -> Res {
    let list = std::mem::take(&mut eng.stomach.cur);
    if eng.mode == -HMODE {
        let p = hpack(eng, list, NATURAL);
        pop_nest(eng);
        append_to_vlist(eng, p);
        if let Some(Node::Box(b)) = eng.stomach.cur.pop() {
            eng.stomach.cur.push(unset_from(*b));
        }
        let adjust = std::mem::take(&mut current(eng).adjust);
        eng.stomach.cur.extend(adjust);
    } else {
        let p = vpack(eng, list, NATURAL);
        pop_nest(eng);
        eng.stomach.cur.push(unset_from(p));
        eng.aux.space_factor = 1000;
    }
    eng.begin_toks_param(EVERY_CR)?;
    align_peek(eng)
}

/// `fin_align` (§800).
fn fin_align(eng: &mut Eng) -> Res {
    if eng.cur_group() != GroupKind::Align {
        return eng.confusion("align1");
    }
    eng.unsave()?;
    if eng.cur_group() != GroupKind::Align {
        return eng.confusion("align0");
    }
    eng.unsave()?;
    let outer_math = eng.stomach.nest.last().is_some_and(|s| s.mode == MMODE);
    let o = if outer_math {
        eng.eqtb().dimen_par(DISPLAY_INDENT)
    } else {
        0
    };
    let Some(mut al) = eng.stomach.aligns.pop() else {
        return Ok(());
    };
    let n_rec = al.records.len();
    for j in 0..n_rec {
        if al.records[j].width == RUNNING {
            al.records[j].width = 0;
            al.glues[j + 1] = tabskip_node(Glue::default(), true);
        }
        let spans = std::mem::take(&mut al.records[j].spans);
        if !spans.is_empty() && j + 1 < n_rec {
            let t = al.records[j].width + glue_spec(&al.glues[j + 1]).0.width;
            for (n, w) in spans {
                let w = w - t;
                if n == 1 {
                    if w > al.records[j + 1].width {
                        al.records[j + 1].width = w;
                    }
                } else {
                    let ps = &mut al.records[j + 1].spans;
                    match ps.iter_mut().find(|(m, _)| *m == n - 1) {
                        Some((_, pw)) => *pw = (*pw).max(w),
                        None => {
                            ps.push((n - 1, w));
                            ps.sort_by_key(|(m, _)| *m);
                        }
                    }
                }
            }
        }
    }
    let mut preamble = Vec::with_capacity(2 * n_rec + 1);
    for j in 0..n_rec {
        preamble.push(al.glues[j].clone());
        preamble.push(Node::Unset(Box::new(UnsetNode {
            width: al.records[j].width,
            depth: 0,
            height: 0,
            span_count: 0,
            stretch: 0,
            shrink: 0,
            stretch_order: GlueOrder::Normal,
            shrink_order: GlueOrder::Normal,
            list: Vec::new(),
        })));
    }
    preamble.push(al.glues[n_rec].clone());
    eng.stomach.pack_begin_line = -eng.stomach.mode_line;
    let vertical = eng.mode == -VMODE;
    let p = if vertical {
        let rule_save = eng.eqtb().dimen_par(OVERFULL_RULE);
        eng.assign_in_place(Loc::DimenPar(OVERFULL_RULE as u16), Value::Int(0));
        let p = hpack(eng, preamble, al.spec);
        eng.assign_in_place(Loc::DimenPar(OVERFULL_RULE as u16), Value::Int(rule_save));
        p
    } else {
        for n in &mut preamble {
            if let Node::Unset(u) = n {
                u.height = u.width;
                u.width = 0;
            }
        }
        let mut p = vpack(eng, preamble, al.spec);
        for n in &mut p.list {
            if let Node::Unset(u) = n {
                u.width = u.height;
                u.height = 0;
            }
        }
        p
    };
    eng.stomach.pack_begin_line = 0;
    let rows = std::mem::take(&mut eng.stomach.cur);
    let mut out = Vec::with_capacity(rows.len());
    for q in rows {
        match q {
            Node::Unset(u) => out.push(set_row(*u, &p, vertical, o, outer_math)),
            Node::Rule { width, depth, height } => {
                let rule = Node::Rule {
                    width: if width == RUNNING { p.width } else { width },
                    depth: if depth == RUNNING { p.depth } else { depth },
                    height: if height == RUNNING { p.height } else { height },
                };
                if o != 0 {
                    let mut b = hpack(eng, vec![rule], NATURAL);
                    b.shift = o;
                    out.push(Node::Box(Box::new(b)));
                } else {
                    out.push(rule);
                }
            }
            other => out.push(other),
        }
    }
    *eng.align_state_mut() = al.saved_align_state;
    let aux_save = (eng.aux.prev_depth, eng.aux.space_factor);
    pop_nest(eng);
    if eng.mode == MMODE {
        return super::math::finish_display_alignment(eng, out, aux_save);
    }
    eng.aux.prev_depth = aux_save.0;
    eng.aux.space_factor = aux_save.1;
    eng.stomach.cur.extend(out);
    if eng.mode == VMODE {
        super::page::build_page(eng)?;
    }
    Ok(())
}

fn set_row(q: UnsetNode, p: &BoxNode, vertical: bool, o: Scaled, display: bool) -> Node {
    let mut row = BoxNode::new(if vertical { BoxDir::H } else { BoxDir::V });
    row.display = vertical && display;
    row.depth = q.depth;
    if vertical {
        row.width = p.width;
        row.height = q.height;
    } else {
        row.height = p.height;
        row.width = q.width;
    }
    row.glue_order = p.glue_order;
    row.glue_sign = p.glue_sign;
    row.glue_set = p.glue_set;
    row.shift = o;
    let proto = &p.list;
    let mut list = Vec::with_capacity(q.list.len());
    let mut items = q.list.into_iter();
    if let Some(first) = items.next() {
        list.push(first);
    }
    let mut s = 1usize;
    while let Some(r) = items.next() {
        let Node::Unset(r) = r else {
            list.push(r);
            continue;
        };
        let mut n = r.span_count;
        let mut t = unset_width(&proto[s]);
        let w = t;
        let mut fillers = Vec::new();
        while n > 0 {
            n -= 1;
            s += 1;
            let (v, zero) = glue_spec(&proto[s]);
            fillers.push(tabskip_node(v, zero));
            t += v.width;
            match p.glue_sign {
                GlueSign::Stretching if v.stretch_order == p.glue_order => {
                    t += (p.glue_set * f64::from(v.stretch)).round() as Scaled;
                }
                GlueSign::Shrinking if v.shrink_order == p.glue_order => {
                    t -= (p.glue_set * f64::from(v.shrink)).round() as Scaled;
                }
                _ => {}
            }
            s += 1;
            let mut empty = BoxNode::new(if vertical { BoxDir::H } else { BoxDir::V });
            if vertical {
                empty.width = unset_width(&proto[s]);
            } else {
                empty.height = unset_width(&proto[s]);
            }
            t += unset_width(&proto[s]);
            fillers.push(Node::Box(Box::new(empty)));
        }
        let mut b = BoxNode::new(if vertical { BoxDir::H } else { BoxDir::V });
        let natural = if vertical { r.width } else { r.height };
        if vertical {
            b.height = row.height;
            b.depth = row.depth;
        } else {
            b.width = row.width;
        }
        b.glue_order = r.stretch_order;
        if t == natural {
            b.glue_sign = GlueSign::Normal;
            b.glue_order = GlueOrder::Normal;
            b.glue_set = 0.0;
        } else if t > natural {
            b.glue_sign = GlueSign::Stretching;
            b.glue_set = if r.stretch == 0 {
                0.0
            } else {
                f64::from(t - natural) / f64::from(r.stretch)
            };
        } else {
            b.glue_order = r.shrink_order;
            b.glue_sign = GlueSign::Shrinking;
            b.glue_set = if r.shrink == 0 {
                0.0
            } else if b.glue_order == GlueOrder::Normal && natural - t > r.shrink {
                1.0
            } else {
                f64::from(natural - t) / f64::from(r.shrink)
            };
        }
        if vertical {
            b.width = w;
        } else {
            b.height = w;
        }
        b.list = r.list;
        list.push(Node::Box(Box::new(b)));
        list.extend(fillers);
        s += 2;
        if let Some(g) = items.next() {
            list.push(g);
        }
    }
    row.list = list;
    Node::Box(Box::new(row))
}

fn unset_width(n: &Node) -> Scaled {
    match n {
        Node::Unset(u) => u.width,
        Node::Box(b) => b.width,
        _ => 0,
    }
}

/// The right brace of an alignment group: a missing `\cr` (§1132).
pub(crate) fn end_align_group(eng: &mut Eng) -> Res {
    eng.back_input()?;
    eng.print_err("Missing ");
    eng.print_esc("cr");
    eng.print_str(" inserted");
    eng.set_help(&["I'm guessing that you meant to end an alignment here."]);
    let cr = eng.frozen_cr_token();
    eng.ins_list(vec![cr])?;
    eng.error()
}

/// The right brace of a `\noalign` group (§1133).
pub(crate) fn end_no_align(eng: &mut Eng) -> Res {
    super::build::end_graf(eng)?;
    eng.unsave()?;
    align_peek(eng)
}

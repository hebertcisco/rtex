//! Building boxes and lists (§1055–§1135).

use std::sync::Arc;

use rtex_core::cmd::{self, chr};
use rtex_core::{CatCode, Glue, GlueOrder, GroupKind, Loc, Res, Token, UNITY, Value};

use super::fonts::{new_character, param};
use super::node::{BoxDir, BoxNode, DiscNode, GlueKind, GlueNode, InsNode, KernKind, MarkNode, Node, RUNNING};
use super::pack::{NATURAL, append_to_vlist, hpack, param_glue, vpack, vpackage};
use super::show::show_box;
use super::{
    BoxContext, Eng, GroupData, HMODE, IGNORE_DEPTH, MMODE, Spec, VMODE, abs_mode, pop_nest, push_nest, tail_append,
    you_cant,
};
use crate::units::Scaled;

const LOOSENESS: usize = 19;
const HANG_AFTER: usize = 41;
const LANGUAGE: usize = 50;
const LEFT_HYPHEN_MIN: usize = 51;
const RIGHT_HYPHEN_MIN: usize = 52;
const FLOATING_PENALTY: usize = 42;
const PAR_INDENT: usize = 0;
const HSIZE: usize = 3;
const BOX_MAX_DEPTH: usize = 7;
const SPLIT_MAX_DEPTH: usize = 6;
const HANG_INDENT: usize = 17;
const PAR_SKIP: usize = 2;
const SPLIT_TOP_SKIP: usize = 10;
const EVERY_PAR: usize = 1;
const EVERY_HBOX: usize = 4;
const EVERY_VBOX: usize = 5;
const DEFAULT_RULE: Scaled = 26_214;

const BOX_CODE: u32 = 0;
const COPY_CODE: u32 = 1;
const LAST_BOX_CODE: u32 = 2;
const VSPLIT_CODE: u32 = 3;
const VTOP_CODE: u32 = 4;

/// The fixed glue specifications behind `\hfil` and friends (§162).
pub(crate) fn fil_glue(code: u32) -> Glue {
    let fil = |stretch: Scaled, order: GlueOrder| Glue {
        stretch,
        stretch_order: order,
        ..Glue::default()
    };
    match code {
        0 => fil(UNITY, GlueOrder::Fil),
        1 => fil(UNITY, GlueOrder::Fill),
        2 => Glue {
            shrink: UNITY,
            shrink_order: GlueOrder::Fil,
            ..fil(UNITY, GlueOrder::Fil)
        },
        _ => fil(-UNITY, GlueOrder::Fil),
    }
}

/// `append_glue` (§1060).
pub(crate) fn append_glue(eng: &mut Eng) -> Res {
    let s = eng.cur_chr();
    let (spec, kind) = match s {
        0..=3 => (fil_glue(s), GlueKind::Normal),
        4 => (*eng.scan_glue(chr::GLUE_VAL)?.get(), GlueKind::Normal),
        _ => (*eng.scan_glue(chr::MU_VAL)?.get(), GlueKind::Mu),
    };
    tail_append(
        eng,
        Node::Glue(GlueNode {
            spec,
            kind,
            leader: None,
            zero_ptr: false,
        }),
    );
    Ok(())
}

/// `append_kern` (§1061).
pub(crate) fn append_kern(eng: &mut Eng) -> Res {
    let mu = eng.cur_chr() == 99;
    let (width, _) = eng.scan_dimen(mu, false, None)?;
    let kind = if mu { KernKind::Mu } else { KernKind::Explicit };
    tail_append(eng, Node::kern(width, kind));
    Ok(())
}

/// `scan_rule_spec` (§463).
pub(crate) fn scan_rule_spec(eng: &mut Eng) -> Res<Node> {
    let (mut width, mut height, mut depth) = if eng.cur_cmd() == cmd::VRULE {
        (DEFAULT_RULE, RUNNING, RUNNING)
    } else {
        (RUNNING, DEFAULT_RULE, 0)
    };
    loop {
        if eng.scan_keyword("width")? {
            width = eng.scan_normal_dimen()?;
        } else if eng.scan_keyword("height")? {
            height = eng.scan_normal_dimen()?;
        } else if eng.scan_keyword("depth")? {
            depth = eng.scan_normal_dimen()?;
        } else {
            return Ok(Node::rule(width, height, depth));
        }
    }
}

/// `vmode+hrule`, `hmode+vrule`, `mmode+vrule` (§1056).
pub(crate) fn append_rule(eng: &mut Eng) -> Res {
    let rule = scan_rule_spec(eng)?;
    tail_append(eng, rule);
    match abs_mode(eng) {
        VMODE => eng.aux.prev_depth = IGNORE_DEPTH,
        HMODE => eng.aux.space_factor = 1000,
        _ => {}
    }
    Ok(())
}

/// `scan_box` (§1084): the next input should specify a box or, for leaders, a rule.
pub(crate) fn scan_box(eng: &mut Eng, context: BoxContext) -> Res {
    eng.get_next_nonblank_nonrelax_x()?;
    if eng.cur_cmd() == cmd::MAKE_BOX {
        begin_box(eng, context)
    } else if matches!(context, BoxContext::Leaders(_)) && matches!(eng.cur_cmd(), cmd::HRULE | cmd::VRULE) {
        let rule = scan_rule_spec(eng)?;
        box_end(eng, context, Some(rule))
    } else {
        eng.print_err("A <box> was supposed to be here");
        eng.set_help(&[
            "I was expecting to see \\hbox or \\vbox or \\copy or \\box or",
            "something like that. So you might find something missing in",
            "your output. But keep trying; you can fix this later.",
        ]);
        eng.back_error()
    }
}

/// The box in register `n`, if any.
pub(crate) fn fetch_box(eng: &Eng, n: u16) -> Option<&BoxNode> {
    eng.stomach.boxes.get(&eng.eqtb().box_reg(n))
}

/// Removes the box from register `n`, leaving it void at the same level (`change_box(null)`).
pub(crate) fn take_box(eng: &mut Eng, n: u16) -> Option<BoxNode> {
    let h = eng.eqtb().box_reg(n);
    if h == 0 {
        return None;
    }
    eng.assign_in_place(Loc::Box(n), Value::Int(0));
    eng.stomach.boxes.remove(&h)
}

/// `define(box_base+n, box_ref, b)` (§1077), releasing a value that is overwritten at its own level.
pub(crate) fn define_box(eng: &mut Eng, n: u16, b: Option<BoxNode>, global: bool) -> Res {
    let old = eng.eqtb().box_reg(n);
    let same_level = eng.eqtb().level(Loc::Box(n)) == eng.cur_level();
    let h = match b {
        Some(b) => eng.stomach.store_box(b),
        None => 0,
    };
    eng.eq_define(Loc::Box(n), Value::Int(h), global)?;
    if old != 0 && old != h && (global || same_level) {
        eng.stomach.boxes.remove(&old);
    }
    Ok(())
}

/// `begin_box` (§1079).
pub(crate) fn begin_box(eng: &mut Eng, context: BoxContext) -> Res {
    let cur_box = match eng.cur_chr() {
        BOX_CODE => {
            let n = eng.scan_register_num()? as u16;
            take_box(eng, n).map(|b| Node::Box(Box::new(b)))
        }
        COPY_CODE => {
            let n = eng.scan_register_num()? as u16;
            fetch_box(eng, n).cloned().map(|b| Node::Box(Box::new(b)))
        }
        LAST_BOX_CODE => last_box(eng)?,
        VSPLIT_CODE => {
            let n = eng.scan_register_num()? as u16;
            if !eng.scan_keyword("to")? {
                eng.print_err("Missing `to' inserted");
                eng.set_help(&[
                    "I'm working on `\\vsplit<box number> to <dimen>';",
                    "will look for the <dimen> next.",
                ]);
                eng.error()?;
            }
            let h = eng.scan_normal_dimen()?;
            super::page::vsplit(eng, n, h)?.map(|b| Node::Box(Box::new(b)))
        }
        c => return initiate_box(eng, context, c),
    };
    box_end(eng, context, cur_box)
}

fn last_box(eng: &mut Eng) -> Res<Option<Node>> {
    if abs_mode(eng) == MMODE {
        you_cant(eng);
        eng.set_help(&["Sorry; this \\lastbox will be void."]);
        eng.error()?;
        return Ok(None);
    }
    if eng.mode == VMODE && eng.stomach.cur.is_empty() {
        you_cant(eng);
        eng.set_help(&[
            "Sorry...I usually can't take things from the current page.",
            "This \\lastbox will therefore be void.",
        ]);
        eng.error()?;
        return Ok(None);
    }
    if !matches!(eng.stomach.cur.last(), Some(Node::Box(_))) || tail_in_discretionary(&eng.stomach.cur) {
        return Ok(None);
    }
    match eng.stomach.cur.pop() {
        Some(Node::Box(mut b)) => {
            b.shift = 0;
            Ok(Some(Node::Box(b)))
        }
        _ => Ok(None),
    }
}

/// Whether the last item of `list` is part of a discretionary's replacement text.
pub(crate) fn tail_in_discretionary(list: &[Node]) -> bool {
    let mut i = 0;
    let last = list.len().saturating_sub(1);
    while i < list.len() {
        if let Node::Disc(d) = &list[i] {
            if d.replace_count > 0 && i < last && i + d.replace_count >= last {
                return true;
            }
            i += d.replace_count;
        }
        i += 1;
    }
    false
}

fn initiate_box(eng: &mut Eng, context: BoxContext, c: u32) -> Res {
    let mut k = c as i32 - VTOP_CODE as i32;
    let (group, vtop) = if k == HMODE {
        let kind = if matches!(context, BoxContext::Shift(_)) && abs_mode(eng) == VMODE {
            GroupKind::AdjustedHBox
        } else {
            GroupKind::HBox
        };
        (kind, false)
    } else if k == VMODE {
        (GroupKind::VBox, false)
    } else {
        k = VMODE;
        (GroupKind::VTop, true)
    };
    let spec = scan_spec(eng)?;
    eng.new_save_level(group)?;
    let level = eng.cur_level();
    eng.stomach.groups.push(GroupData::Box {
        context,
        spec,
        vtop,
        level,
    });
    eng.scan_left_brace()?;
    if k == VMODE {
        normal_paragraph(eng)?;
    }
    push_nest(eng);
    eng.mode = -k;
    if k == VMODE {
        eng.aux.prev_depth = IGNORE_DEPTH;
        eng.begin_toks_param(EVERY_VBOX)
    } else {
        eng.aux.space_factor = 1000;
        eng.begin_toks_param(EVERY_HBOX)
    }
}

/// `scan_spec` (§645) without the group: `to` or `spread`.
pub(crate) fn scan_spec(eng: &mut Eng) -> Res<Spec> {
    Ok(if eng.scan_keyword("to")? {
        Spec::Exactly(eng.scan_normal_dimen()?)
    } else if eng.scan_keyword("spread")? {
        Spec::Additional(eng.scan_normal_dimen()?)
    } else {
        NATURAL
    })
}

/// `package` (§1086), at the right brace of a box group.
pub(crate) fn package(eng: &mut Eng) -> Res {
    let d = eng.eqtb().dimen_par(BOX_MAX_DEPTH);
    eng.unsave()?;
    let Some(GroupData::Box {
        context, spec, vtop, ..
    }) = eng.stomach.groups.pop()
    else {
        return Ok(());
    };
    let restricted_h = eng.mode == -HMODE;
    let list = std::mem::take(&mut eng.stomach.cur);
    let b = if restricted_h {
        hpack(eng, list, spec)
    } else {
        let mut b = vpackage(eng, list, spec, d);
        if vtop {
            let h = match b.list.first() {
                Some(Node::Box(f)) => f.height,
                Some(Node::Rule { height, .. }) => *height,
                _ => 0,
            };
            b.depth = b.depth - h + b.height;
            b.height = h;
        }
        b
    };
    pop_nest(eng);
    box_end(eng, context, Some(Node::Box(Box::new(b))))
}

/// `box_end` (§1075).
pub(crate) fn box_end(eng: &mut Eng, context: BoxContext, cur_box: Option<Node>) -> Res {
    match context {
        BoxContext::Shift(amount) => {
            let Some(mut node) = cur_box else { return Ok(()) };
            if let Node::Box(b) = &mut node {
                b.shift = amount;
            }
            if abs_mode(eng) == VMODE {
                if let Node::Box(b) = node {
                    append_to_vlist(eng, *b);
                } else {
                    tail_append(eng, node);
                }
                if let Some(adjust) = eng.stomach.adjust.take() {
                    eng.stomach.cur.extend(adjust);
                }
                if eng.mode > 0 {
                    super::page::build_page(eng)?;
                }
            } else if abs_mode(eng) == HMODE {
                eng.aux.space_factor = 1000;
                tail_append(eng, node);
            } else {
                super::math::append_box_noad(eng, node);
            }
            Ok(())
        }
        BoxContext::SetBox { register, global } => {
            let b = match cur_box {
                Some(Node::Box(b)) => Some(*b),
                _ => None,
            };
            define_box(eng, register, b, global)
        }
        BoxContext::ShipOut => match cur_box {
            Some(Node::Box(b)) => super::shipout::ship_out(eng, *b),
            _ => Ok(()),
        },
        BoxContext::Leaders(kind) => {
            let Some(node) = cur_box else { return Ok(()) };
            eng.get_next_nonblank_nonrelax_x()?;
            let proper = (eng.cur_cmd() == cmd::HSKIP && abs_mode(eng) != VMODE)
                || (eng.cur_cmd() == cmd::VSKIP && abs_mode(eng) == VMODE);
            if proper {
                append_glue(eng)?;
                if let Some(Node::Glue(g)) = eng.stomach.cur.last_mut() {
                    g.kind = kind;
                    g.leader = Some(Box::new(node));
                }
                Ok(())
            } else {
                eng.print_err("Leaders not followed by proper glue");
                eng.set_help(&[
                    "You should say `\\leaders <box or rule><hskip or vskip>'.",
                    "I found the <box or rule>, but there's no suitable",
                    "<hskip or vskip>, so I'm ignoring these leaders.",
                ]);
                eng.back_error()
            }
        }
    }
}

/// `normal_paragraph` (§1070).
pub(crate) fn normal_paragraph(eng: &mut Eng) -> Res {
    if eng.int_par(LOOSENESS) != 0 {
        eng.eq_define(Loc::IntPar(LOOSENESS as u16), Value::Int(0), false)?;
    }
    if eng.eqtb().dimen_par(HANG_INDENT) != 0 {
        eng.eq_define(Loc::DimenPar(HANG_INDENT as u16), Value::Int(0), false)?;
    }
    if eng.int_par(HANG_AFTER) != 1 {
        eng.eq_define(Loc::IntPar(HANG_AFTER as u16), Value::Int(1), false)?;
    }
    if eng.eqtb().shape(0).is_some() {
        eng.eq_define(Loc::Shape(0), Value::Shape(None), false)?;
    }
    if eng.eqtb().shape(1).is_some() {
        eng.eq_define(Loc::Shape(1), Value::Shape(None), false)?;
    }
    Ok(())
}

fn norm_min(h: i32) -> i32 {
    h.clamp(1, 63)
}

/// `cur_lang` from `\language` (§934).
pub(crate) fn cur_lang(eng: &Eng) -> i32 {
    let l = eng.int_par(LANGUAGE);
    if (1..=255).contains(&l) { l } else { 0 }
}

/// `new_graf` (§1091).
pub(crate) fn new_graf(eng: &mut Eng, indented: bool) -> Res {
    eng.stomach.prev_graf = 0;
    if eng.mode == VMODE || !eng.stomach.cur.is_empty() {
        let glue = param_glue(eng, PAR_SKIP);
        tail_append(eng, glue);
    }
    push_nest(eng);
    eng.mode = HMODE;
    eng.aux.space_factor = 1000;
    let lang = cur_lang(eng);
    eng.stomach.clang = lang;
    eng.stomach.prev_graf =
        (norm_min(eng.int_par(LEFT_HYPHEN_MIN)) * 0o100 + norm_min(eng.int_par(RIGHT_HYPHEN_MIN))) * 0o200000 + lang;
    if indented {
        let mut b = BoxNode::new(BoxDir::H);
        b.width = eng.eqtb().dimen_par(PAR_INDENT);
        tail_append(eng, Node::Box(Box::new(b)));
    }
    eng.begin_toks_param(EVERY_PAR)?;
    if eng.stomach.nest.len() == 1 {
        super::page::build_page(eng)?;
    }
    Ok(())
}

/// `indent_in_hmode` (§1093).
pub(crate) fn indent_in_hmode(eng: &mut Eng) -> Res {
    if eng.cur_chr() > 0 {
        let mut b = BoxNode::new(BoxDir::H);
        b.width = eng.eqtb().dimen_par(PAR_INDENT);
        if abs_mode(eng) == HMODE {
            eng.aux.space_factor = 1000;
            tail_append(eng, Node::Box(Box::new(b)));
        } else {
            super::math::append_box_noad(eng, Node::Box(Box::new(b)));
        }
    }
    Ok(())
}

/// `head_for_vmode` (§1095).
pub(crate) fn head_for_vmode(eng: &mut Eng) -> Res {
    if eng.mode < 0 {
        if eng.cur_cmd() != cmd::HRULE {
            eng.off_save()
        } else {
            eng.print_err("You can't use `");
            eng.print_esc("hrule");
            eng.print_str("' here except with leaders");
            eng.set_help(&[
                "To put a horizontal rule in an hbox or an alignment,",
                "you should use \\leaders or \\hrulefill (see The TeXbook).",
            ]);
            eng.error()
        }
    } else {
        eng.back_input()?;
        let par = eng.par_token();
        eng.ins_list(vec![par])
    }
}

/// `end_graf` (§1096).
pub(crate) fn end_graf(eng: &mut Eng) -> Res {
    if eng.mode == HMODE {
        if eng.stomach.cur.is_empty() {
            pop_nest(eng);
        } else {
            super::linebreak::line_break(eng, false)?;
        }
        normal_paragraph(eng)?;
        eng.reset_error_count();
    }
    Ok(())
}

/// `begin_insert_or_adjust` (§1099).
pub(crate) fn begin_insert_or_adjust(eng: &mut Eng) -> Res {
    let number = if eng.cur_cmd() == cmd::VADJUST {
        255
    } else {
        let n = eng.scan_eight_bit_int()?;
        if n == 255 {
            eng.print_err("You can't ");
            eng.print_esc("insert");
            eng.print_int(255);
            eng.set_help(&["I'm changing to \\insert0; box 255 is special."]);
            eng.error()?;
            0
        } else {
            n as u8
        }
    };
    eng.stomach.groups.push(GroupData::Insert { number });
    eng.new_save_level(GroupKind::Insert)?;
    eng.scan_left_brace()?;
    normal_paragraph(eng)?;
    push_nest(eng);
    eng.mode = -VMODE;
    eng.aux.prev_depth = IGNORE_DEPTH;
    Ok(())
}

/// The right brace of an insert group (§1100).
pub(crate) fn finish_insert(eng: &mut Eng) -> Res {
    end_graf(eng)?;
    let split_top = *eng.eqtb().glue_par(SPLIT_TOP_SKIP).get();
    let depth = eng.eqtb().dimen_par(SPLIT_MAX_DEPTH);
    let cost = eng.int_par(FLOATING_PENALTY);
    eng.unsave()?;
    let number = match eng.stomach.groups.pop() {
        Some(GroupData::Insert { number }) => number,
        _ => 255,
    };
    let list = std::mem::take(&mut eng.stomach.cur);
    let p = vpack(eng, list, NATURAL);
    pop_nest(eng);
    if number < 255 {
        tail_append(
            eng,
            Node::Ins(Box::new(InsNode {
                number,
                height: p.height + p.depth,
                depth,
                split_top,
                float_cost: cost,
                list: p.list,
            })),
        );
    } else {
        tail_append(eng, Node::Adjust(p.list));
    }
    if eng.stomach.nest.is_empty() {
        super::page::build_page(eng)?;
    }
    Ok(())
}

/// `make_mark` (§1101).
pub(crate) fn make_mark(eng: &mut Eng) -> Res {
    let class = if eng.cur_chr() == 0 {
        0
    } else {
        eng.scan_register_num()?
    };
    let toks = eng.scan_token_list(true)?;
    tail_append(
        eng,
        Node::Mark(MarkNode {
            class,
            toks: Arc::from(toks),
        }),
    );
    Ok(())
}

/// `append_penalty` (§1103).
pub(crate) fn append_penalty(eng: &mut Eng) -> Res {
    let p = eng.scan_int()?;
    tail_append(eng, Node::Penalty(p));
    if eng.mode == VMODE {
        super::page::build_page(eng)?;
    }
    Ok(())
}

/// `delete_last` (§1105): `\unpenalty`, `\unkern`, `\unskip`.
pub(crate) fn delete_last(eng: &mut Eng) -> Res {
    let c = eng.cur_chr();
    if eng.mode == VMODE && eng.stomach.cur.is_empty() {
        if c != 10 || eng.stomach.page.last_glue.is_some() {
            you_cant(eng);
            let last = match c {
                11 => "Try `I\\kern-\\lastkern' instead.",
                10 => "Try `I\\vskip-\\lastskip' instead.",
                _ => "Perhaps you can make the output routine do it.",
            };
            eng.set_help(&["Sorry...I usually can't take things from the current page.", last]);
            eng.error()?;
        }
        return Ok(());
    }
    let matches = match eng.stomach.cur.last() {
        Some(Node::Glue(_)) => c == 10,
        Some(Node::Kern { .. }) => c == 11,
        Some(Node::Penalty(_)) => c == 12,
        _ => false,
    };
    if matches && !tail_in_discretionary(&eng.stomach.cur) {
        eng.stomach.cur.pop();
    }
    Ok(())
}

/// `unpackage` (§1110).
pub(crate) fn unpackage(eng: &mut Eng) -> Res {
    let c = eng.cur_chr();
    if c > COPY_CODE {
        let saved = std::mem::take(&mut eng.stomach.disc_ptr[c as usize]);
        eng.stomach.cur.extend(saved);
        return Ok(());
    }
    let n = eng.scan_register_num()? as u16;
    let Some(dir) = fetch_box(eng, n).map(|b| b.dir) else {
        return Ok(());
    };
    let m = abs_mode(eng);
    if m == MMODE || (m == VMODE && dir != BoxDir::V) || (m == HMODE && dir != BoxDir::H) {
        eng.print_err("Incompatible list can't be unboxed");
        eng.set_help(&[
            "Sorry, Pandora. (You sneaky devil.)",
            "I refuse to unbox an \\hbox in vertical mode or vice versa.",
            "And I can't open any boxes in math mode.",
        ]);
        return eng.error();
    }
    let list = if c == COPY_CODE {
        fetch_box(eng, n).map(|b| b.list.clone()).unwrap_or_default()
    } else {
        take_box(eng, n).map(|b| b.list).unwrap_or_default()
    };
    eng.stomach.cur.extend(list);
    Ok(())
}

/// `append_italic_correction` (§1113).
pub(crate) fn append_italic_correction(eng: &mut Eng) -> Res {
    let (f, c) = match eng.stomach.cur.last() {
        Some(Node::Char { font, ch }) => (*font, *ch),
        Some(Node::Ligature(l)) => (l.font, l.ch),
        _ => return Ok(()),
    };
    let w = eng.stomach.fonts.italic(f, c);
    tail_append(eng, Node::kern(w, KernKind::Explicit));
    Ok(())
}

/// `append_discretionary` (§1117).
pub(crate) fn append_discretionary(eng: &mut Eng) -> Res {
    let mut disc = DiscNode {
        pre: Vec::new(),
        post: Vec::new(),
        replace_count: 0,
    };
    if eng.cur_chr() == 1 {
        let f = eng.eqtb().cur_font() as usize;
        let c = eng.fonts().get(f).map_or(-1, |i| i.hyphen_char);
        if (0..256).contains(&c)
            && let Some(n) = new_character(eng, f, c as u32)
        {
            disc.pre.push(n);
        }
        tail_append(eng, Node::Disc(Box::new(disc)));
        return Ok(());
    }
    tail_append(eng, Node::Disc(Box::new(disc)));
    eng.stomach.groups.push(GroupData::Disc { part: 0 });
    eng.new_save_level(GroupKind::Disc)?;
    eng.scan_left_brace()?;
    push_nest(eng);
    eng.mode = -HMODE;
    eng.aux.space_factor = 1000;
    Ok(())
}

/// `build_discretionary` (§1119).
pub(crate) fn build_discretionary(eng: &mut Eng) -> Res {
    eng.unsave()?;
    let mut list = std::mem::take(&mut eng.stomach.cur);
    let bad = list.iter().position(|n| {
        !matches!(
            n,
            Node::Char { .. } | Node::Box(_) | Node::Rule { .. } | Node::Kern { .. } | Node::Ligature(_)
        )
    });
    if let Some(i) = bad {
        eng.print_err("Improper discretionary list");
        eng.set_help(&["Discretionary lists must contain only boxes and kerns."]);
        eng.error()?;
        eng.begin_diagnostic();
        eng.print_nl("The following discretionary sublist has been deleted:");
        let tail = list.split_off(i);
        show_box(eng, &tail);
        eng.end_diagnostic(true);
    }
    let n = list.len();
    pop_nest(eng);
    let part = match eng.stomach.groups.pop() {
        Some(GroupData::Disc { part }) => part,
        _ => 2,
    };
    match part {
        0 | 1 => {
            if let Some(Node::Disc(d)) = eng.stomach.cur.last_mut() {
                if part == 0 {
                    d.pre = list;
                } else {
                    d.post = list;
                }
            }
            eng.stomach.groups.push(GroupData::Disc { part: part + 1 });
            eng.new_save_level(GroupKind::Disc)?;
            eng.scan_left_brace()?;
            push_nest(eng);
            eng.mode = -HMODE;
            eng.aux.space_factor = 1000;
        }
        _ => {
            if n > 0 && abs_mode(eng) == MMODE {
                eng.print_err("Illegal math ");
                eng.print_esc("discretionary");
                eng.set_help(&[
                    "Sorry: The third part of a discretionary break must be",
                    "empty, in math formulas. I had to delete your third part.",
                ]);
                eng.error()?;
            } else {
                if let Some(Node::Disc(d)) = eng.stomach.cur.last_mut() {
                    d.replace_count = n;
                }
                eng.stomach.cur.extend(list);
            }
        }
    }
    Ok(())
}

/// `make_accent` (§1123).
pub(crate) fn make_accent(eng: &mut Eng) -> Res {
    let c = eng.scan_char_num()? as u32;
    let f = eng.eqtb().cur_font() as usize;
    let Some(p) = new_character(eng, f, c) else {
        return Ok(());
    };
    let x = param(eng, f, 4);
    let s = f64::from(param(eng, f, 0)) / 65536.0;
    let a = eng.stomach.fonts.width(f, c);
    eng.do_assignments()?;
    let f = eng.eqtb().cur_font() as usize;
    let q = match eng.cur_cmd() {
        cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN => new_character(eng, f, eng.cur_chr()),
        cmd::CHAR_NUM => {
            let c = eng.scan_char_num()? as u32;
            new_character(eng, f, c)
        }
        _ => {
            eng.back_input()?;
            None
        }
    };
    let mut items = Vec::new();
    match q {
        Some(Node::Char { ch: qc, .. }) => {
            let t = f64::from(param(eng, f, 0)) / 65536.0;
            let w = eng.stomach.fonts.width(f, qc);
            let h = eng.stomach.fonts.height(f, qc);
            let accent = if h != x {
                let mut b = hpack(eng, vec![p], NATURAL);
                b.shift = x - h;
                Node::Box(Box::new(b))
            } else {
                p
            };
            let delta = (f64::from(w - a) / 2.0 + f64::from(h) * t - f64::from(x) * s).round() as Scaled;
            items.push(Node::kern(delta, KernKind::Accent));
            items.push(accent);
            items.push(Node::kern(-a - delta, KernKind::Accent));
            items.push(Node::Char { font: f, ch: qc });
        }
        _ => items.push(p),
    }
    eng.stomach.cur.extend(items);
    eng.aux.space_factor = 1000;
    Ok(())
}

/// `align_error` (§1127).
pub(crate) fn align_error(eng: &mut Eng) -> Res {
    let state = *eng.align_state_mut();
    if state.abs() > 2 {
        eng.print_err("Misplaced ");
        eng.print_cmd_chr(eng.cur_cmd(), eng.cur_chr());
        if eng.cur_tok().cat() == Some(CatCode::AlignTab) {
            eng.set_help(&[
                "I can't figure out why you would want to use a tab mark",
                "here. If you just want an ampersand, the remedy is",
                "simple: Just type `I\\&' now. But if some right brace",
                "up above has ended a previous alignment prematurely,",
                "you're probably due for more error messages, and you",
                "might try typing `S' now just to see what is salvageable.",
            ]);
        } else {
            eng.set_help(&[
                "I can't figure out why you would want to use a tab mark",
                "or \\cr or \\span just now. If something like a right brace",
                "up above has ended a previous alignment prematurely,",
                "you're probably due for more error messages, and you",
                "might try typing `S' now just to see what is salvageable.",
            ]);
        }
        return eng.error();
    }
    eng.back_input()?;
    let tok = if state < 0 {
        eng.print_err("Missing { inserted");
        *eng.align_state_mut() += 1;
        Token::char(CatCode::BeginGroup, u32::from('{'))
    } else {
        eng.print_err("Missing } inserted");
        *eng.align_state_mut() -= 1;
        Token::char(CatCode::EndGroup, u32::from('}'))
    };
    eng.set_help(&[
        "I've put in what seems to be necessary to fix",
        "the current column of the current alignment.",
        "Try to go on, since this might almost work.",
    ]);
    eng.ins_list(vec![tok])?;
    eng.error()
}

/// `no_align_error` and `omit_error` (§1129).
pub(crate) fn misplaced(eng: &mut Eng, name: &str) -> Res {
    eng.print_err("Misplaced ");
    eng.print_esc(name);
    if name == "noalign" {
        eng.set_help(&[
            "I expect to see \\noalign only after the \\cr of",
            "an alignment. Proceed, and I'll ignore this case.",
        ]);
    } else {
        eng.set_help(&[
            "I expect to see \\omit only after tab marks or the \\cr of",
            "an alignment. Proceed, and I'll ignore this case.",
        ]);
    }
    eng.error()
}

/// `insert_dollar_sign` (§1047).
pub(crate) fn insert_dollar_sign(eng: &mut Eng) -> Res {
    eng.back_input()?;
    eng.print_err("Missing $ inserted");
    eng.set_help(&[
        "I've inserted a begin-math/end-math symbol since I think",
        "you left one out. Proceed, with fingers crossed.",
    ]);
    eng.ins_list(vec![Token::char(CatCode::MathShift, u32::from('$'))])?;
    eng.error()
}

/// `its_all_over` (§1054).
pub(crate) fn its_all_over(eng: &mut Eng) -> Res<bool> {
    if eng.mode <= 0 {
        eng.report_illegal_case()?;
        return Ok(false);
    }
    if eng.stomach.page.list.is_empty() && eng.stomach.cur.is_empty() && eng.aux.dead_cycles == 0 {
        return Ok(true);
    }
    eng.back_input()?;
    let mut b = BoxNode::new(BoxDir::H);
    b.width = eng.eqtb().dimen_par(HSIZE);
    tail_append(eng, Node::Box(Box::new(b)));
    tail_append(eng, Node::glue(fil_glue(1)));
    tail_append(eng, Node::Penalty(-0o10000000000));
    super::page::build_page(eng)?;
    Ok(false)
}

/// `\showbox` (§1296).
pub(crate) fn show_box_register(eng: &mut Eng) -> Res {
    let n = eng.scan_register_num()?;
    eng.begin_diagnostic();
    eng.print_nl("> \\box");
    eng.print_int(n);
    eng.print_char(u32::from('='));
    match fetch_box(eng, n as u16).cloned() {
        None => eng.print_str("void"),
        Some(b) => super::show::show_box_node(eng, &Node::Box(Box::new(b))),
    }
    Ok(())
}

/// `\wd`, `\ht`, `\dp` assignment (`alter_box_dimen`, §1247).
pub(crate) fn alter_box_dimen(eng: &mut Eng) -> Res {
    let c = eng.cur_chr();
    let n = eng.scan_register_num()? as u16;
    eng.scan_optional_equals()?;
    let d = eng.scan_normal_dimen()?;
    let h = eng.eqtb().box_reg(n);
    if let Some(b) = eng.stomach.boxes.get_mut(&h) {
        match c {
            1 => b.width = d,
            3 => b.height = d,
            _ => b.depth = d,
        }
    }
    Ok(())
}

/// `\setbox` (§1241).
pub(crate) fn set_box(eng: &mut Eng, global: bool) -> Res {
    let n = eng.scan_register_num()? as u16;
    eng.scan_optional_equals()?;
    if eng.set_box_allowed() {
        scan_box(eng, BoxContext::SetBox { register: n, global })
    } else {
        eng.print_err("Improper ");
        eng.print_esc("setbox");
        eng.set_help(&[
            "Sorry, \\setbox is not allowed after \\halign in a display,",
            "or between \\accent and an accented character.",
        ]);
        eng.error()
    }
}

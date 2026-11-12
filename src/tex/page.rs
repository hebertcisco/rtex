//! Breaking vertical lists into pages: `vsplit`, the page builder and `fire_up` (§967–§1028).

use std::sync::Arc;

use rtex_core::{GlueOrder, GroupKind, Loc, Res, Token, TokenKind, Value, badness, x_over_n};

use super::build::{fetch_box, normal_paragraph};
use super::node::{BoxDir, BoxNode, EJECT_PENALTY, INF_BAD, INF_PENALTY, InsNode, MAX_DIMEN, Node};
use super::pack::{NATURAL, skip_param, vpack, vpackage};
use super::show::show_box_node;
use super::{Eng, GroupData, IGNORE_DEPTH, Spec, VMODE, pop_nest, push_nest};
use crate::units::Scaled;

pub(crate) const AWFUL_BAD: i32 = 0o7777777777;
const DEPLORABLE: i32 = 100_000;

const VBADNESS: usize = 27;
const VFUZZ: usize = 9;
const VSIZE: usize = 4;
const MAX_DEPTH: usize = 5;
const SPLIT_MAX_DEPTH: usize = 6;
const TOP_SKIP: usize = 9;
const SPLIT_TOP_SKIP: usize = 10;
const TRACING_PAGES: usize = 33;
const OUTPUT_PENALTY: usize = 39;
const MAX_DEAD_CYCLES: usize = 40;
const HOLDING_INSERTS: usize = 53;
const SAVING_VDISCARDS: usize = 62;
const OUTPUT_ROUTINE: usize = 0;

/// What the current page holds (§980).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PageContents {
    /// Nothing yet.
    #[default]
    Empty,
    /// Only insertions.
    InsertsOnly,
    /// A box or rule has been contributed.
    BoxThere,
}

/// A page insertion record (§981).
#[derive(Clone, Debug)]
pub(crate) struct PageIns {
    pub number: u8,
    pub split_up: bool,
    pub height: Scaled,
    pub broken_ptr: Option<usize>,
    pub broken_ins: Option<usize>,
    pub last_ins_ptr: Option<usize>,
    pub best_ins_ptr: Option<usize>,
}

fn is_running_glue_or_kern(n: &Node) -> bool {
    matches!(n, Node::Glue(_) | Node::Kern { .. } | Node::Penalty(_))
}

/// `prune_page_top` (§968); discarded items are kept when `save` is set (e-TeX).
pub(crate) fn prune_page_top(eng: &Eng, list: Vec<Node>, save: bool) -> (Vec<Node>, Vec<Node>) {
    let mut out = Vec::with_capacity(list.len());
    let mut discarded = Vec::new();
    let mut iter = list.into_iter();
    while let Some(n) = iter.next() {
        match &n {
            Node::Box(_) | Node::Rule { .. } => {
                let h = match &n {
                    Node::Box(b) => b.height,
                    Node::Rule { height, .. } => *height,
                    _ => 0,
                };
                let top = eng.eqtb().glue_par(SPLIT_TOP_SKIP).get().width;
                out.push(skip_param(eng, SPLIT_TOP_SKIP, if top > h { top - h } else { 0 }));
                out.push(n);
                out.extend(iter);
                return (out, discarded);
            }
            Node::Whatsit(_) | Node::Mark(_) | Node::Ins(_) => out.push(n),
            _ if is_running_glue_or_kern(&n) => {
                if save {
                    discarded.push(n);
                }
            }
            _ => out.push(n),
        }
    }
    (out, discarded)
}

/// `vert_break` (§970): the best break index in `list` for height `h` and depth `d`
/// (`None` is the end), with the natural height of the best box.
pub(crate) fn vert_break(eng: &mut Eng, list: &mut [Node], h: Scaled, d: Scaled) -> Res<(Option<usize>, Scaled)> {
    let mut prev_p: Option<usize> = None;
    let mut least_cost = AWFUL_BAD;
    let mut active = [0; 7];
    let mut prev_dp = 0;
    let mut best_place = None;
    let mut best_hd = 0;
    let mut p = 0usize;
    loop {
        let pi;
        let mut update = false;
        let mut not_found = false;
        if p >= list.len() {
            pi = EJECT_PENALTY;
        } else {
            match &list[p] {
                Node::Box(b) => {
                    active[1] += prev_dp + b.height;
                    prev_dp = b.depth;
                    not_found = true;
                    pi = 0;
                }
                Node::Rule { height, depth, .. } => {
                    active[1] += prev_dp + height;
                    prev_dp = *depth;
                    not_found = true;
                    pi = 0;
                }
                Node::Whatsit(_) | Node::Mark(_) | Node::Ins(_) => {
                    if let Some((_, ht, dp)) = eng.stomach.backend.node_size(&list[p]) {
                        active[1] += prev_dp + ht;
                        prev_dp = dp;
                    }
                    not_found = true;
                    pi = 0;
                }
                Node::Glue(_) => {
                    if prev_p.is_some_and(|q| list[q].precedes_break()) {
                        pi = 0;
                    } else {
                        update = true;
                        pi = 0;
                    }
                }
                Node::Kern { .. } => {
                    if matches!(list.get(p + 1), Some(Node::Glue(_))) {
                        pi = 0;
                    } else {
                        update = true;
                        pi = 0;
                    }
                }
                Node::Penalty(x) => pi = *x,
                _ => {
                    not_found = true;
                    pi = 0;
                }
            }
        }
        if !not_found && !update && pi < INF_PENALTY {
            let cur = active[1];
            let b = if cur < h {
                if active[3] != 0 || active[4] != 0 || active[5] != 0 {
                    0
                } else {
                    badness(h - cur, active[2])
                }
            } else if cur - h > active[6] {
                AWFUL_BAD
            } else {
                badness(cur - h, active[6])
            };
            let b = if b < AWFUL_BAD {
                if pi <= EJECT_PENALTY {
                    pi
                } else if b < INF_BAD {
                    b + pi
                } else {
                    DEPLORABLE
                }
            } else {
                b
            };
            if b <= least_cost {
                best_place = if p >= list.len() { None } else { Some(p) };
                least_cost = b;
                best_hd = active[1] + prev_dp;
            }
            if b == AWFUL_BAD || pi <= EJECT_PENALTY {
                return Ok((best_place, best_hd));
            }
        }
        if !not_found && p < list.len() && matches!(list[p], Node::Glue(_) | Node::Kern { .. }) {
            match &mut list[p] {
                Node::Kern { width, .. } => {
                    active[1] += prev_dp + *width;
                }
                Node::Glue(g) => {
                    active[2 + g.spec.stretch_order as usize] += g.spec.stretch;
                    active[6] += g.spec.shrink;
                    let infinite = g.spec.shrink_order != GlueOrder::Normal && g.spec.shrink != 0;
                    g.spec.shrink_order = GlueOrder::Normal;
                    let w = g.spec.width;
                    if infinite {
                        eng.print_err("Infinite glue shrinkage found in box being split");
                        eng.set_help(&[
                            "The box you are \\vsplitting contains some infinitely",
                            "shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
                            "Such glue doesn't belong there; but you can safely proceed,",
                            "since the offensive shrinkability has been made finite.",
                        ]);
                        eng.error()?;
                    }
                    active[1] += prev_dp + w;
                }
                _ => {}
            }
            prev_dp = 0;
        }
        if prev_dp > d {
            active[1] += prev_dp - d;
            prev_dp = d;
        }
        prev_p = Some(p);
        p += 1;
    }
}

fn update_split_marks(eng: &mut Eng, list: &[Node]) {
    for n in list {
        if let Node::Mark(m) = n {
            if eng.stomach.marks.get(m.class, 3).is_none() {
                eng.stomach.marks.set(m.class, 3, Some(m.toks.clone()));
            }
            eng.stomach.marks.set(m.class, 4, Some(m.toks.clone()));
        }
    }
}

/// `vsplit` (§977).
pub(crate) fn vsplit(eng: &mut Eng, n: u16, h: Scaled) -> Res<Option<BoxNode>> {
    let classes: Vec<i32> = eng.stomach.marks.classes.keys().copied().collect();
    for c in classes {
        eng.stomach.marks.set(c, 3, None);
        eng.stomach.marks.set(c, 4, None);
    }
    let Some(v) = fetch_box(eng, n).cloned() else {
        return Ok(None);
    };
    if v.dir != BoxDir::V {
        eng.print_err("");
        eng.print_esc("vsplit");
        eng.print_str(" needs a ");
        eng.print_esc("vbox");
        eng.set_help(&[
            "The box you are trying to split is an \\hbox.",
            "I can't split such a box, so I'll leave it alone.",
        ]);
        eng.error()?;
        return Ok(None);
    }
    let d = eng.eqtb().dimen_par(SPLIT_MAX_DEPTH);
    let mut list = v.list;
    let (q, _) = vert_break(eng, &mut list, h, d)?;
    let split_at = q.unwrap_or(list.len());
    let rest = list.split_off(split_at);
    update_split_marks(eng, &list);
    let save = eng.int_par(SAVING_VDISCARDS) > 0;
    let (rest, discarded) = prune_page_top(eng, rest, save);
    if save {
        eng.stomach.disc_ptr[3] = discarded;
    }
    let h_reg = eng.eqtb().box_reg(n);
    eng.stomach.boxes.remove(&h_reg);
    if rest.is_empty() {
        eng.assign_in_place(Loc::Box(n), Value::Int(0));
    } else {
        let b = vpack(eng, rest, NATURAL);
        let handle = eng.stomach.store_box(b);
        eng.assign_in_place(Loc::Box(n), Value::Int(handle));
    }
    Ok(Some(vpackage(eng, list, Spec::Exactly(h), d)))
}

/// `print_totals` (§985).
pub(crate) fn print_totals(eng: &mut Eng) {
    let s = eng.aux.page_so_far;
    eng.print_scaled(s[1]);
    for (i, unit) in [(2, ""), (3, "fil"), (4, "fill"), (5, "filll")] {
        if s[i] != 0 {
            eng.print_str(" plus ");
            eng.print_scaled(s[i]);
            eng.print_str(unit);
        }
    }
    if s[6] != 0 {
        eng.print_str(" minus ");
        eng.print_scaled(s[6]);
    }
}

/// The insertion lines of `\showlists` (§986).
pub(crate) fn show_page_inserts(eng: &mut Eng) {
    let inserts = eng.stomach.page.inserts.clone();
    for r in inserts {
        eng.print_ln();
        eng.print_esc("insert");
        let t = i32::from(r.number);
        eng.print_int(t);
        eng.print_str(" adds ");
        let count = eng.eqtb().count(r.number.into());
        let amount = if count == 1000 {
            r.height
        } else {
            x_over_n(r.height, 1000).quotient * count
        };
        eng.print_scaled(amount);
        if r.split_up {
            let until = r.broken_ins.unwrap_or(0);
            let t = eng.stomach.page.list[..=until.min(eng.stomach.page.list.len().saturating_sub(1))]
                .iter()
                .filter(|n| matches!(n, Node::Ins(i) if i.number == r.number))
                .count();
            eng.print_str(", #");
            eng.print_int(t as i32);
            eng.print_str(" might split");
        }
    }
}

fn freeze_page_specs(eng: &mut Eng, s: PageContents) {
    eng.stomach.page.contents = s;
    eng.aux.page_so_far = [0; 8];
    eng.aux.page_so_far[0] = eng.eqtb().dimen_par(VSIZE);
    eng.stomach.page.max_depth = eng.eqtb().dimen_par(MAX_DEPTH);
    eng.stomach.page.least_cost = AWFUL_BAD;
    if eng.int_par(TRACING_PAGES) > 0 {
        eng.begin_diagnostic();
        eng.print_nl("%% goal height=");
        eng.print_scaled(eng.aux.page_so_far[0]);
        eng.print_str(", max depth=");
        eng.print_scaled(eng.stomach.page.max_depth);
        eng.end_diagnostic(false);
    }
}

fn box_error(eng: &mut Eng, n: u16) -> Res {
    eng.error()?;
    eng.begin_diagnostic();
    eng.print_nl("The following box has been deleted:");
    if let Some(b) = fetch_box(eng, n).cloned() {
        show_box_node(eng, &Node::Box(Box::new(b)));
    }
    eng.end_diagnostic(true);
    let h = eng.eqtb().box_reg(n);
    eng.stomach.boxes.remove(&h);
    eng.assign_in_place(Loc::Box(n), Value::Int(0));
    Ok(())
}

fn ensure_vbox(eng: &mut Eng, n: u16) -> Res {
    if fetch_box(eng, n).is_some_and(|b| b.dir == BoxDir::H) {
        eng.print_err("Insertions can only be added to a vbox");
        eng.set_help(&[
            "Tut tut: You're trying to \\insert into a",
            "\\box register that now contains an \\hbox.",
            "Proceed, and I'll discard its present contents.",
        ]);
        box_error(eng, n)?;
    }
    Ok(())
}

fn ins_height(eng: &Eng, n: u8, h: Scaled) -> Scaled {
    let count = eng.eqtb().count(n.into());
    if count == 1000 {
        h
    } else {
        x_over_n(h, 1000).quotient * count
    }
}

fn start_new_page(eng: &mut Eng) {
    let p = &mut eng.stomach.page;
    p.contents = PageContents::Empty;
    p.list.clear();
    p.last_glue = None;
    p.last_penalty = 0;
    p.last_kern = 0;
    p.last_node_type = -1;
    p.max_depth = 0;
    eng.aux.page_so_far[7] = 0;
}

/// `build_page` (§994).
pub(crate) fn build_page(eng: &mut Eng) -> Res {
    if eng.stomach.contrib().is_empty() || eng.stomach.page.output_active {
        return Ok(());
    }
    while let Some(p) = eng.stomach.contrib().first().cloned() {
        {
            let page = &mut eng.stomach.page;
            page.last_penalty = 0;
            page.last_kern = 0;
            page.last_node_type = p.type_code();
            page.last_glue = None;
            match &p {
                Node::Glue(g) => page.last_glue = Some(g.spec),
                Node::Penalty(x) => page.last_penalty = *x,
                Node::Kern { width, .. } => page.last_kern = *width,
                _ => {}
            }
        }
        let contents = eng.stomach.page.contents;
        enum Next {
            Contribute,
            Update,
            Break(i32),
            Drop,
            Continue,
        }
        let next = match &p {
            Node::Box(_) | Node::Rule { .. } => {
                if contents < PageContents::BoxThere {
                    if contents == PageContents::Empty {
                        freeze_page_specs(eng, PageContents::BoxThere);
                    } else {
                        eng.stomach.page.contents = PageContents::BoxThere;
                    }
                    let h = match &p {
                        Node::Box(b) => b.height,
                        Node::Rule { height, .. } => *height,
                        _ => 0,
                    };
                    let top = eng.eqtb().glue_par(TOP_SKIP).get().width;
                    let q = skip_param(eng, TOP_SKIP, if top > h { top - h } else { 0 });
                    eng.stomach.contrib().insert(0, q);
                    Next::Continue
                } else {
                    let (h, d) = match &p {
                        Node::Box(b) => (b.height, b.depth),
                        Node::Rule { height, depth, .. } => (*height, *depth),
                        _ => (0, 0),
                    };
                    eng.aux.page_so_far[1] += eng.aux.page_so_far[7] + h;
                    eng.aux.page_so_far[7] = d;
                    Next::Contribute
                }
            }
            Node::Whatsit(_) => {
                if let Some((_, h, d)) = eng.stomach.backend.node_size(&p) {
                    eng.aux.page_so_far[1] += eng.aux.page_so_far[7] + h;
                    eng.aux.page_so_far[7] = d;
                }
                Next::Contribute
            }
            Node::Mark(_) => Next::Contribute,
            Node::Glue(_) => {
                if contents < PageContents::BoxThere {
                    Next::Drop
                } else if eng.stomach.page.list.last().is_some_and(Node::precedes_break) {
                    Next::Break(0)
                } else {
                    Next::Update
                }
            }
            Node::Kern { .. } => {
                if contents < PageContents::BoxThere {
                    Next::Drop
                } else {
                    match eng.stomach.contrib().get(1) {
                        None => return Ok(()),
                        Some(Node::Glue(_)) => Next::Break(0),
                        Some(_) => Next::Update,
                    }
                }
            }
            Node::Penalty(x) => {
                if contents < PageContents::BoxThere {
                    Next::Drop
                } else {
                    Next::Break(*x)
                }
            }
            Node::Ins(ins) => {
                append_insertion(eng, ins)?;
                Next::Contribute
            }
            _ => Next::Contribute,
        };
        match next {
            Next::Continue => continue,
            Next::Drop => {
                let p = eng.stomach.contrib().remove(0);
                if eng.int_par(SAVING_VDISCARDS) > 0 {
                    eng.stomach.disc_ptr[1].push(p);
                }
                continue;
            }
            Next::Break(pi) => {
                if pi < INF_PENALTY {
                    let s = eng.aux.page_so_far;
                    let b = if s[1] < s[0] {
                        if s[3] != 0 || s[4] != 0 || s[5] != 0 {
                            0
                        } else {
                            badness(s[0] - s[1], s[2])
                        }
                    } else if s[1] - s[0] > s[6] {
                        AWFUL_BAD
                    } else {
                        badness(s[1] - s[0], s[6])
                    };
                    let ip = eng.aux.insert_penalties;
                    let mut c = if b < AWFUL_BAD {
                        if pi <= EJECT_PENALTY {
                            pi
                        } else if b < INF_BAD {
                            b + pi + ip
                        } else {
                            DEPLORABLE
                        }
                    } else {
                        b
                    };
                    if ip >= 10000 {
                        c = AWFUL_BAD;
                    }
                    if eng.int_par(TRACING_PAGES) > 0 {
                        eng.begin_diagnostic();
                        eng.print_nl("%");
                        eng.print_str(" t=");
                        print_totals(eng);
                        eng.print_str(" g=");
                        eng.print_scaled(s[0]);
                        eng.print_str(" b=");
                        if b == AWFUL_BAD {
                            eng.print_char(u32::from('*'));
                        } else {
                            eng.print_int(b);
                        }
                        eng.print_str(" p=");
                        eng.print_int(pi);
                        eng.print_str(" c=");
                        if c == AWFUL_BAD {
                            eng.print_char(u32::from('*'));
                        } else {
                            eng.print_int(c);
                        }
                        if c <= eng.stomach.page.least_cost {
                            eng.print_char(u32::from('#'));
                        }
                        eng.end_diagnostic(false);
                    }
                    if c <= eng.stomach.page.least_cost {
                        let page = &mut eng.stomach.page;
                        page.best_break = Some(page.list.len());
                        page.best_size = s[0];
                        page.least_cost = c;
                        for r in &mut page.inserts {
                            r.best_ins_ptr = r.last_ins_ptr;
                        }
                    }
                    if c == AWFUL_BAD || pi <= EJECT_PENALTY {
                        fire_up(eng)?;
                        if eng.stomach.page.output_active {
                            return Ok(());
                        }
                        if eng.stomach.contrib().is_empty() {
                            break;
                        }
                        continue;
                    }
                }
                if matches!(p, Node::Glue(_) | Node::Kern { .. }) {
                    update_heights(eng)?;
                }
            }
            Next::Update => update_heights(eng)?,
            Next::Contribute => {}
        }
        if eng.aux.page_so_far[7] > eng.stomach.page.max_depth {
            eng.aux.page_so_far[1] += eng.aux.page_so_far[7] - eng.stomach.page.max_depth;
            eng.aux.page_so_far[7] = eng.stomach.page.max_depth;
        }
        let p = eng.stomach.contrib().remove(0);
        eng.stomach.page.list.push(p);
        if eng.stomach.contrib().is_empty() {
            break;
        }
    }
    Ok(())
}

fn update_heights(eng: &mut Eng) -> Res {
    let p = eng.stomach.contrib().first().cloned();
    let width = match p {
        Some(Node::Kern { width, .. }) => width,
        Some(Node::Glue(g)) => {
            let s = &mut eng.aux.page_so_far;
            s[2 + g.spec.stretch_order as usize] += g.spec.stretch;
            s[6] += g.spec.shrink;
            if g.spec.shrink_order != GlueOrder::Normal && g.spec.shrink != 0 {
                eng.print_err("Infinite glue shrinkage found on current page");
                eng.set_help(&[
                    "The page about to be output contains some infinitely",
                    "shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
                    "Such glue doesn't belong there; but you can safely proceed,",
                    "since the offensive shrinkability has been made finite.",
                ]);
                eng.error()?;
                if let Some(Node::Glue(gn)) = eng.stomach.contrib().first_mut() {
                    gn.spec.shrink_order = GlueOrder::Normal;
                }
            }
            g.spec.width
        }
        _ => 0,
    };
    eng.aux.page_so_far[1] += eng.aux.page_so_far[7] + width;
    eng.aux.page_so_far[7] = 0;
    Ok(())
}

fn append_insertion(eng: &mut Eng, ins: &InsNode) -> Res {
    if eng.stomach.page.contents == PageContents::Empty {
        freeze_page_specs(eng, PageContents::InsertsOnly);
    }
    let n = ins.number;
    let pos = eng.stomach.page.inserts.iter().position(|r| r.number >= n);
    let idx = match pos {
        Some(i) if eng.stomach.page.inserts[i].number == n => i,
        _ => {
            let i = pos.unwrap_or(eng.stomach.page.inserts.len());
            ensure_vbox(eng, n.into())?;
            let height = fetch_box(eng, n.into()).map_or(0, |b| b.height + b.depth);
            eng.stomach.page.inserts.insert(
                i,
                PageIns {
                    number: n,
                    split_up: false,
                    height,
                    broken_ptr: None,
                    broken_ins: None,
                    last_ins_ptr: None,
                    best_ins_ptr: None,
                },
            );
            let q = *eng.eqtb().skip(n.into()).get();
            let h = ins_height(eng, n, height);
            let s = &mut eng.aux.page_so_far;
            s[0] = s[0] - h - q.width;
            s[2 + q.stretch_order as usize] += q.stretch;
            s[6] += q.shrink;
            if q.shrink_order != GlueOrder::Normal && q.shrink != 0 {
                eng.print_err("Infinite glue shrinkage inserted from ");
                eng.print_esc("skip");
                eng.print_int(i32::from(n));
                eng.set_help(&[
                    "The correction glue for page breaking with insertions",
                    "must have finite shrinkability. But you may proceed,",
                    "since the offensive shrinkability has been made finite.",
                ]);
                eng.error()?;
            }
            i
        }
    };
    if eng.stomach.page.inserts[idx].split_up {
        eng.aux.insert_penalties += ins.float_cost;
        return Ok(());
    }
    let page_index = eng.stomach.page.list.len();
    eng.stomach.page.inserts[idx].last_ins_ptr = Some(page_index);
    let s = eng.aux.page_so_far;
    let delta = s[0] - s[1] - s[7] + s[6];
    let h = ins_height(eng, n, ins.height);
    let r_height = eng.stomach.page.inserts[idx].height;
    let dimen_n = eng.eqtb().dimen(n.into());
    if (h <= 0 || h <= delta) && ins.height + r_height <= dimen_n {
        eng.aux.page_so_far[0] -= h;
        eng.stomach.page.inserts[idx].height += ins.height;
        return Ok(());
    }
    let count = eng.eqtb().count(n.into());
    let mut w = if count <= 0 {
        MAX_DIMEN
    } else {
        let w = s[0] - s[1] - s[7];
        if count != 1000 {
            x_over_n(w, count).quotient * 1000
        } else {
            w
        }
    };
    if w > dimen_n - r_height {
        w = dimen_n - r_height;
    }
    let mut list = ins.list.clone();
    let (q, mut best_hd) = vert_break(eng, &mut list, w, ins.depth)?;
    eng.stomach.page.inserts[idx].height += best_hd;
    if eng.int_par(TRACING_PAGES) > 0 {
        eng.begin_diagnostic();
        eng.print_nl("% split");
        eng.print_int(i32::from(n));
        eng.print_str(" to ");
        eng.print_scaled(w);
        eng.print_char(u32::from(','));
        eng.print_scaled(best_hd);
        eng.print_str(" p=");
        match q.map(|i| &list[i]) {
            None => eng.print_int(EJECT_PENALTY),
            Some(Node::Penalty(x)) => eng.print_int(*x),
            Some(_) => eng.print_char(u32::from('0')),
        }
        eng.end_diagnostic(false);
    }
    if count != 1000 {
        best_hd = x_over_n(best_hd, 1000).quotient * count;
    }
    eng.aux.page_so_far[0] -= best_hd;
    let r = &mut eng.stomach.page.inserts[idx];
    r.split_up = true;
    r.broken_ptr = q;
    r.broken_ins = Some(page_index);
    match q.map(|i| &list[i]) {
        None => eng.aux.insert_penalties += EJECT_PENALTY,
        Some(Node::Penalty(x)) => eng.aux.insert_penalties += *x,
        Some(_) => {}
    }
    Ok(())
}

fn update_fire_marks(eng: &mut Eng, list: &[Node]) {
    let classes: Vec<i32> = eng.stomach.marks.classes.keys().copied().collect();
    for c in classes {
        if let Some(bot) = eng.stomach.marks.get(c, 2) {
            eng.stomach.marks.set(c, 0, Some(bot));
            eng.stomach.marks.set(c, 1, None);
        }
    }
    for n in list {
        if let Node::Mark(m) = n {
            if eng.stomach.marks.get(m.class, 1).is_none() {
                eng.stomach.marks.set(m.class, 1, Some(m.toks.clone()));
            }
            eng.stomach.marks.set(m.class, 2, Some(m.toks.clone()));
        }
    }
    let classes: Vec<i32> = eng.stomach.marks.classes.keys().copied().collect();
    for c in classes {
        if eng.stomach.marks.get(c, 0).is_some() && eng.stomach.marks.get(c, 1).is_none() {
            let top = eng.stomach.marks.get(c, 0);
            eng.stomach.marks.set(c, 1, top);
        }
    }
}

/// `fire_up` (§1012): `c` is the node at the front of the contribution list.
fn fire_up(eng: &mut Eng) -> Res {
    let best = eng.stomach.page.best_break.unwrap_or(eng.stomach.page.list.len());
    let page_len = eng.stomach.page.list.len();
    let penalty = if best < page_len {
        match &mut eng.stomach.page.list[best] {
            Node::Penalty(x) => Some(std::mem::replace(x, INF_PENALTY)),
            _ => None,
        }
    } else {
        match eng.stomach.contrib().first_mut() {
            Some(Node::Penalty(x)) => Some(std::mem::replace(x, INF_PENALTY)),
            _ => None,
        }
    };
    eng.eq_define(
        Loc::IntPar(OUTPUT_PENALTY as u16),
        Value::Int(penalty.unwrap_or(INF_PENALTY)),
        true,
    )?;
    if eng.eqtb().box_reg(255) != 0 {
        eng.print_err("");
        eng.print_esc("box");
        eng.print_str("255 is not void");
        eng.set_help(&[
            "You shouldn't use \\box255 except in \\output routines.",
            "Proceed, and I'll discard its present contents.",
        ]);
        box_error(eng, 255)?;
    }
    eng.aux.insert_penalties = 0;
    let save_split_top = eng.eqtb().glue_par(SPLIT_TOP_SKIP).clone();
    let holding = eng.int_par(HOLDING_INSERTS) > 0;
    let mut page = std::mem::take(&mut eng.stomach.page.list);
    let rest = page.split_off(best.min(page.len()));
    update_fire_marks(eng, &page);
    let inserts = std::mem::take(&mut eng.stomach.page.inserts);
    let mut kept = Vec::with_capacity(page.len());
    let mut held = Vec::new();
    if !holding {
        let mut queues: Vec<(u8, Vec<Node>)> = Vec::new();
        for r in &inserts {
            if r.best_ins_ptr.is_some() {
                ensure_vbox(eng, r.number.into())?;
                let existing = super::build::take_box(eng, r.number.into())
                    .map(|b| b.list)
                    .unwrap_or_default();
                queues.push((r.number, existing));
            }
        }
        for (i, node) in page.into_iter().enumerate() {
            let Node::Ins(mut ins) = node else {
                kept.push(node);
                continue;
            };
            let Some(r) = inserts.iter().find(|r| r.number == ins.number) else {
                kept.push(Node::Ins(ins));
                continue;
            };
            let mut wait = r.best_ins_ptr.is_none();
            if !wait {
                let queue = &mut queues.iter_mut().find(|(n, _)| *n == ins.number).expect("queue").1;
                let start = queue.len();
                queue.extend(std::mem::take(&mut ins.list));
                if r.best_ins_ptr == Some(i) {
                    if r.split_up
                        && r.broken_ins == Some(i)
                        && let Some(bp) = r.broken_ptr
                    {
                        let remainder = queue.split_off(start + bp);
                        eng.assign_in_place(
                            Loc::GluePar(SPLIT_TOP_SKIP as u16),
                            Value::Glue(rtex_core::GlueRef(Arc::new(ins.split_top))),
                        );
                        let (pruned, _) = prune_page_top(eng, remainder, false);
                        if !pruned.is_empty() {
                            let tb = vpack(eng, pruned.clone(), NATURAL);
                            ins.height = tb.height + tb.depth;
                            ins.list = pruned;
                            wait = true;
                        }
                    }
                    let list = std::mem::take(queue);
                    let b = vpack(eng, list, NATURAL);
                    let h = eng.stomach.store_box(b);
                    eng.assign_in_place(Loc::Box(ins.number.into()), Value::Int(h));
                    queue.clear();
                }
            }
            if wait {
                held.push(Node::Ins(ins));
                eng.aux.insert_penalties += 1;
            }
        }
        for (n, list) in queues {
            if !list.is_empty() {
                let b = vpack(eng, list, NATURAL);
                let h = eng.stomach.store_box(b);
                eng.assign_in_place(Loc::Box(n.into()), Value::Int(h));
            }
        }
    } else {
        kept = page;
    }
    eng.assign_in_place(Loc::GluePar(SPLIT_TOP_SKIP as u16), Value::Glue(save_split_top));
    if !rest.is_empty() {
        let contrib = eng.stomach.contrib();
        let tail = std::mem::take(contrib);
        contrib.extend(rest);
        contrib.extend(tail);
    }
    let save_vbadness = eng.int_par(VBADNESS);
    let save_vfuzz = eng.eqtb().dimen_par(VFUZZ);
    eng.assign_in_place(Loc::IntPar(VBADNESS as u16), Value::Int(INF_BAD));
    eng.assign_in_place(Loc::DimenPar(VFUZZ as u16), Value::Int(MAX_DIMEN));
    let best_size = eng.stomach.page.best_size;
    let max_depth = eng.stomach.page.max_depth;
    let b255 = vpackage(eng, kept, Spec::Exactly(best_size), max_depth);
    eng.assign_in_place(Loc::IntPar(VBADNESS as u16), Value::Int(save_vbadness));
    eng.assign_in_place(Loc::DimenPar(VFUZZ as u16), Value::Int(save_vfuzz));
    let h = eng.stomach.store_box(b255);
    eng.assign_in_place(Loc::Box(255), Value::Int(h));
    start_new_page(eng);
    eng.stomach.page.best_break = None;
    eng.stomach.page.list = held;
    let output = eng.eqtb().toks_par(OUTPUT_ROUTINE).clone();
    if let Some(toks) = output.filter(|t| !t.is_empty()) {
        if eng.aux.dead_cycles >= eng.int_par(MAX_DEAD_CYCLES) {
            eng.print_err("Output loop---");
            eng.print_int(eng.aux.dead_cycles);
            eng.print_str(" consecutive dead cycles");
            eng.set_help(&[
                "I've concluded that your \\output is awry; it never does a",
                "\\shipout, so I'm shipping \\box255 out myself. Next time",
                "increase \\maxdeadcycles if you want me to be more patient!",
            ]);
            eng.error()?;
        } else {
            eng.stomach.page.output_active = true;
            eng.aux.dead_cycles += 1;
            push_nest(eng);
            eng.mode = -VMODE;
            eng.aux.prev_depth = IGNORE_DEPTH;
            eng.stomach.mode_line = -eng.line();
            eng.begin_token_list(toks, TokenKind::OutputText)?;
            eng.stomach.groups.push(GroupData::Output);
            eng.new_save_level(GroupKind::Output)?;
            normal_paragraph(eng)?;
            eng.scan_left_brace()?;
            return Ok(());
        }
    }
    if !eng.stomach.page.list.is_empty() {
        let held = std::mem::take(&mut eng.stomach.page.list);
        let contrib = eng.stomach.contrib();
        let tail = std::mem::take(contrib);
        contrib.extend(held);
        contrib.extend(tail);
    }
    if let Some(b) = super::build::take_box(eng, 255) {
        super::shipout::ship_out(eng, b)?;
    }
    Ok(())
}

/// The right brace of an output routine (§1100).
pub(crate) fn resume_after_output(eng: &mut Eng) -> Res {
    if !eng.output_text_ended() {
        eng.print_err("Unbalanced output routine");
        eng.set_help(&[
            "Your sneaky output routine has problematic {'s and/or }'s.",
            "I can't handle that very well; good luck.",
        ]);
        eng.error()?;
        eng.skip_to_end_of_token_list()?;
    }
    eng.end_current_token_list()?;
    super::build::end_graf(eng)?;
    eng.unsave()?;
    eng.stomach.groups.pop();
    eng.stomach.page.output_active = false;
    eng.aux.insert_penalties = 0;
    if eng.eqtb().box_reg(255) != 0 {
        eng.print_err("Output routine didn't use all of ");
        eng.print_esc("box");
        eng.print_int(255);
        eng.set_help(&[
            "Your \\output commands should empty \\box255,",
            "e.g., by saying `\\shipout\\box255'.",
            "Proceed; I'll discard its present contents.",
        ]);
        box_error(eng, 255)?;
    }
    let made = std::mem::take(&mut eng.stomach.cur);
    eng.stomach.page.list.extend(made);
    let page = std::mem::take(&mut eng.stomach.page.list);
    pop_nest(eng);
    if !page.is_empty() {
        let contrib = eng.stomach.contrib();
        let tail = std::mem::take(contrib);
        contrib.extend(page);
        contrib.extend(tail);
    }
    build_page(eng)
}

/// A mark text for `\topmark` and friends.
pub(crate) fn mark_text(eng: &Eng, code: u32, class: i32) -> Option<Arc<[Token]>> {
    eng.stomach.marks.get(class, code as usize)
}

/// The height of insertion class `n` on the current page, as `\pdfinsertht` reports it.
pub(crate) fn insert_height(eng: &Eng, n: i32) -> Scaled {
    eng.stomach
        .page
        .inserts
        .iter()
        .find(|i| i32::from(i.number) == n)
        .map_or(0, |i| i.height)
}

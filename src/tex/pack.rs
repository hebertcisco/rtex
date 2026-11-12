//! Packaging: `hpack`, `vpackage` and `append_to_vlist` (§644–§679).

use rtex_core::{GlueOrder, badness};

use super::node::{BoxDir, BoxNode, GlueKind, GlueNode, GlueSign, Node, RUNNING};
use super::show::{short_display, show_box_node};
use super::{Eng, IGNORE_DEPTH, Spec};
use crate::units::Scaled;

const HBADNESS: usize = 26;
const VBADNESS: usize = 27;
const HFUZZ: usize = 8;
const VFUZZ: usize = 9;
const OVERFULL_RULE: usize = 16;
const LINE_SKIP: usize = 0;
const BASELINE_SKIP: usize = 1;
const LINE_SKIP_LIMIT: usize = 2;

#[derive(Default)]
struct Totals {
    stretch: [Scaled; 4],
    shrink: [Scaled; 4],
}

impl Totals {
    fn add(&mut self, g: &rtex_core::Glue) {
        self.stretch[g.stretch_order as usize] += g.stretch;
        self.shrink[g.shrink_order as usize] += g.shrink;
    }

    fn order(a: &[Scaled; 4]) -> GlueOrder {
        if a[3] != 0 {
            GlueOrder::Filll
        } else if a[2] != 0 {
            GlueOrder::Fill
        } else if a[1] != 0 {
            GlueOrder::Fil
        } else {
            GlueOrder::Normal
        }
    }
}

/// The natural size of a packed list: `natural` means `additional 0`.
pub(crate) const NATURAL: Spec = Spec::Additional(0);

/// `hpack` (§649); vertical material inside goes to `eng.stomach.adjust` when it is collecting.
pub(crate) fn hpack(eng: &mut Eng, mut list: Vec<Node>, spec: Spec) -> BoxNode {
    eng.stomach.last_badness = 0;
    let mut h = 0;
    let mut d = 0;
    let mut x = 0;
    let mut t = Totals::default();
    let collecting = eng.stomach.adjust.is_some();
    let mut i = 0;
    while i < list.len() {
        match &list[i] {
            Node::Char { font, ch } => {
                let fonts = &eng.stomach.fonts;
                x += fonts.width(*font, *ch);
                h = h.max(fonts.height(*font, *ch));
                d = d.max(fonts.depth(*font, *ch));
            }
            Node::Ligature(l) => {
                let fonts = &eng.stomach.fonts;
                x += fonts.width(l.font, l.ch);
                h = h.max(fonts.height(l.font, l.ch));
                d = d.max(fonts.depth(l.font, l.ch));
            }
            Node::Box(b) => {
                x += b.width;
                h = h.max(b.height - b.shift);
                d = d.max(b.depth + b.shift);
            }
            Node::Rule { width, depth, height } => {
                x += width;
                h = h.max(*height);
                d = d.max(*depth);
            }
            Node::Unset(u) => {
                x += u.width;
                h = h.max(u.height);
                d = d.max(u.depth);
            }
            Node::Ins(_) | Node::Mark(_) | Node::Adjust(_) if collecting => {
                let node = list.remove(i);
                let adjust = eng.stomach.adjust.get_or_insert_with(Vec::new);
                match node {
                    Node::Adjust(inner) => adjust.extend(inner),
                    other => adjust.push(other),
                }
                continue;
            }
            Node::Glue(g) => {
                x += g.spec.width;
                t.add(&g.spec);
                if let Some(leader) = &g.leader {
                    let (lh, ld) = node_height_depth(leader);
                    h = h.max(lh);
                    d = d.max(ld);
                }
            }
            Node::Kern { width, .. } | Node::Math { width, .. } | Node::MarginKern { width, .. } => x += width,
            n => {
                if let Some((w, ht, dp)) = eng.stomach.backend.node_size(n) {
                    x += w;
                    h = h.max(ht);
                    d = d.max(dp);
                }
            }
        }
        i += 1;
    }
    eng.stomach.pack_totals = (t.stretch, t.shrink);
    let mut r = BoxNode::new(BoxDir::H);
    r.height = h;
    r.depth = d;
    let w = match spec {
        Spec::Additional(s) => x + s,
        Spec::Exactly(s) => s,
    };
    r.width = w;
    r.list = list;
    let excess = w - x;
    if excess == 0 {
        return r;
    }
    let report = if excess > 0 {
        let o = Totals::order(&t.stretch);
        r.glue_order = o;
        r.glue_sign = GlueSign::Stretching;
        if t.stretch[o as usize] != 0 {
            r.glue_set = f64::from(excess) / f64::from(t.stretch[o as usize]);
        } else {
            r.glue_sign = GlueSign::Normal;
            r.glue_set = 0.0;
        }
        if o == GlueOrder::Normal && !r.list.is_empty() {
            eng.stomach.last_badness = badness(excess, t.stretch[0]);
            let b = eng.stomach.last_badness;
            if b > eng.int_par(HBADNESS) {
                eng.print_ln();
                eng.print_nl(if b > 100 { "Underfull" } else { "Loose" });
                eng.print_str(" \\hbox (badness ");
                eng.print_int(b);
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        let o = Totals::order(&t.shrink);
        r.glue_order = o;
        r.glue_sign = GlueSign::Shrinking;
        if t.shrink[o as usize] != 0 {
            r.glue_set = f64::from(-excess) / f64::from(t.shrink[o as usize]);
        } else {
            r.glue_sign = GlueSign::Normal;
            r.glue_set = 0.0;
        }
        if t.shrink[o as usize] < -excess && o == GlueOrder::Normal && !r.list.is_empty() {
            eng.stomach.last_badness = 1_000_000;
            r.glue_set = 1.0;
            let over = -excess - t.shrink[0];
            let hfuzz = eng.eqtb().dimen_par(HFUZZ);
            if over > hfuzz || eng.int_par(HBADNESS) < 100 {
                let rule = eng.eqtb().dimen_par(OVERFULL_RULE);
                if rule > 0 && over > hfuzz {
                    r.list.push(Node::rule(rule, RUNNING, RUNNING));
                }
                eng.print_ln();
                eng.print_nl("Overfull \\hbox (");
                eng.print_scaled(over);
                eng.print_str("pt too wide");
                true
            } else {
                false
            }
        } else if o == GlueOrder::Normal && !r.list.is_empty() {
            eng.stomach.last_badness = badness(-excess, t.shrink[0]);
            let b = eng.stomach.last_badness;
            if b > eng.int_par(HBADNESS) {
                eng.print_ln();
                eng.print_nl("Tight \\hbox (badness ");
                eng.print_int(b);
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if report {
        finish_hbox_report(eng, &r);
    }
    r
}

fn finish_hbox_report(eng: &mut Eng, r: &BoxNode) {
    if eng.stomach.page.output_active {
        eng.print_str(") has occurred while \\output is active");
    } else {
        let pbl = eng.stomach.pack_begin_line;
        if pbl != 0 {
            eng.print_str(if pbl > 0 {
                ") in paragraph at lines "
            } else {
                ") in alignment at lines "
            });
            eng.print_int(pbl.abs());
            eng.print_str("--");
        } else {
            eng.print_str(") detected at line ");
        }
        eng.print_int(eng.line());
    }
    eng.print_ln();
    eng.stomach.font_in_short_display = 0;
    short_display(eng, &r.list);
    eng.print_ln();
    eng.begin_diagnostic();
    show_box_node(eng, &Node::Box(Box::new(r.clone())));
    eng.end_diagnostic(true);
}

fn node_height_depth(n: &Node) -> (Scaled, Scaled) {
    match n {
        Node::Box(b) => (b.height, b.depth),
        Node::Rule { height, depth, .. } => (*height, *depth),
        _ => (0, 0),
    }
}

fn node_width(n: &Node) -> Scaled {
    match n {
        Node::Box(b) => b.width,
        Node::Rule { width, .. } => *width,
        _ => 0,
    }
}

/// `vpackage` (§668): `max_depth` limits the depth of the result.
pub(crate) fn vpackage(eng: &mut Eng, list: Vec<Node>, spec: Spec, max_depth: Scaled) -> BoxNode {
    eng.stomach.last_badness = 0;
    let mut w = 0;
    let mut d = 0;
    let mut x = 0;
    let mut t = Totals::default();
    for node in &list {
        match node {
            Node::Box(b) => {
                x += d + b.height;
                d = b.depth;
                w = w.max(b.width + b.shift);
            }
            Node::Rule { width, depth, height } => {
                x += d + height;
                d = *depth;
                w = w.max(*width);
            }
            Node::Unset(u) => {
                x += d + u.height;
                d = u.depth;
                w = w.max(u.width);
            }
            Node::Glue(g) => {
                x += d;
                d = 0;
                x += g.spec.width;
                t.add(&g.spec);
                if let Some(leader) = &g.leader {
                    w = w.max(node_width(leader));
                }
            }
            Node::Kern { width, .. } => {
                x += d + width;
                d = 0;
            }
            n => {
                if let Some((wd, ht, dp)) = eng.stomach.backend.node_size(n) {
                    x += d + ht;
                    d = dp;
                    w = w.max(wd);
                }
            }
        }
    }
    eng.stomach.pack_totals = (t.stretch, t.shrink);
    let mut r = BoxNode::new(BoxDir::V);
    r.width = w;
    if d > max_depth {
        x += d - max_depth;
        r.depth = if max_depth >= 0 { max_depth } else { 0 };
    } else {
        r.depth = d;
    }
    let h = match spec {
        Spec::Additional(s) => x + s,
        Spec::Exactly(s) => s,
    };
    r.height = h;
    r.list = list;
    let excess = h - x;
    if excess == 0 {
        return r;
    }
    let report = if excess > 0 {
        let o = Totals::order(&t.stretch);
        r.glue_order = o;
        r.glue_sign = GlueSign::Stretching;
        if t.stretch[o as usize] != 0 {
            r.glue_set = f64::from(excess) / f64::from(t.stretch[o as usize]);
        } else {
            r.glue_sign = GlueSign::Normal;
            r.glue_set = 0.0;
        }
        if o == GlueOrder::Normal && !r.list.is_empty() {
            eng.stomach.last_badness = badness(excess, t.stretch[0]);
            let b = eng.stomach.last_badness;
            if b > eng.int_par(VBADNESS) {
                eng.print_ln();
                eng.print_nl(if b > 100 { "Underfull" } else { "Loose" });
                eng.print_str(" \\vbox (badness ");
                eng.print_int(b);
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        let o = Totals::order(&t.shrink);
        r.glue_order = o;
        r.glue_sign = GlueSign::Shrinking;
        if t.shrink[o as usize] != 0 {
            r.glue_set = f64::from(-excess) / f64::from(t.shrink[o as usize]);
        } else {
            r.glue_sign = GlueSign::Normal;
            r.glue_set = 0.0;
        }
        if t.shrink[o as usize] < -excess && o == GlueOrder::Normal && !r.list.is_empty() {
            eng.stomach.last_badness = 1_000_000;
            r.glue_set = 1.0;
            let over = -excess - t.shrink[0];
            if over > eng.eqtb().dimen_par(VFUZZ) || eng.int_par(VBADNESS) < 100 {
                eng.print_ln();
                eng.print_nl("Overfull \\vbox (");
                eng.print_scaled(over);
                eng.print_str("pt too high");
                true
            } else {
                false
            }
        } else if o == GlueOrder::Normal && !r.list.is_empty() {
            eng.stomach.last_badness = badness(-excess, t.shrink[0]);
            let b = eng.stomach.last_badness;
            if b > eng.int_par(VBADNESS) {
                eng.print_ln();
                eng.print_nl("Tight \\vbox (badness ");
                eng.print_int(b);
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if report {
        if eng.stomach.page.output_active {
            eng.print_str(") has occurred while \\output is active");
        } else {
            let pbl = eng.stomach.pack_begin_line;
            if pbl != 0 {
                eng.print_str(") in alignment at lines ");
                eng.print_int(pbl.abs());
                eng.print_str("--");
            } else {
                eng.print_str(") detected at line ");
            }
            eng.print_int(eng.line());
            eng.print_ln();
        }
        eng.begin_diagnostic();
        show_box_node(eng, &Node::Box(Box::new(r.clone())));
        eng.end_diagnostic(true);
    }
    r
}

/// `vpack` without a depth limit (`vpack(p,h,m)=vpackage(p,h,m,max_dimen)`).
pub(crate) fn vpack(eng: &mut Eng, list: Vec<Node>, spec: Spec) -> BoxNode {
    vpackage(eng, list, spec, super::node::MAX_DIMEN)
}

/// A glue node for parameter `i`, sharing its specification (`new_param_glue`, §152).
pub(crate) fn param_glue(eng: &Eng, i: usize) -> Node {
    let g = eng.eqtb().glue_par(i);
    let spec = *g.get();
    let zero = std::sync::Arc::ptr_eq(&g.0, &eng.eqtb().zero_glue().0);
    Node::Glue(GlueNode {
        spec,
        kind: GlueKind::Param(i as u8),
        leader: None,
        zero_ptr: zero,
    })
}

/// A glue node for parameter `i` with its own copy of the specification (`new_skip_param`, §154).
pub(crate) fn skip_param(eng: &Eng, i: usize, width: Scaled) -> Node {
    let spec = rtex_core::Glue {
        width,
        ..*eng.eqtb().glue_par(i).get()
    };
    Node::Glue(GlueNode {
        spec,
        kind: GlueKind::Param(i as u8),
        leader: None,
        zero_ptr: false,
    })
}

/// `append_to_vlist` (§679).
pub(crate) fn append_to_vlist(eng: &mut Eng, b: BoxNode) {
    if eng.aux.prev_depth > IGNORE_DEPTH {
        let d = eng.eqtb().glue_par(BASELINE_SKIP).get().width - eng.aux.prev_depth - b.height;
        let glue = if d < eng.eqtb().dimen_par(LINE_SKIP_LIMIT) {
            param_glue(eng, LINE_SKIP)
        } else {
            skip_param(eng, BASELINE_SKIP, d)
        };
        eng.stomach.cur.push(glue);
    }
    eng.aux.prev_depth = b.depth;
    eng.stomach.cur.push(Node::Box(Box::new(b)));
}

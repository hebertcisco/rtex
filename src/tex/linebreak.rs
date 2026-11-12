//! Breaking paragraphs into lines: Knuth and Plass's algorithm as in §813–§890.

use rtex_core::{Glue, GlueOrder, Res, badness};

use super::hyphen;
use super::node::{EJECT_PENALTY, GlueKind, INF_BAD, INF_PENALTY, KernKind, MathEdge, Node, Whatsit};
use super::pack::{append_to_vlist, hpack, param_glue};
use super::show::short_display;
use super::{Eng, Spec, pop_nest};
use crate::units::Scaled;

pub(crate) const AWFUL_BAD: i32 = 0o7777777777;
const MAX_HALFWORD: i32 = 0xFFFFFFF;

const PRETOLERANCE: usize = 0;
const TOLERANCE: usize = 1;
const LINE_PENALTY: usize = 2;
const HYPHEN_PENALTY: usize = 3;
const EX_HYPHEN_PENALTY: usize = 4;
const CLUB_PENALTY: usize = 5;
const WIDOW_PENALTY: usize = 6;
const DISPLAY_WIDOW_PENALTY: usize = 7;
const BROKEN_PENALTY: usize = 8;
const INTER_LINE_PENALTY: usize = 13;
const DOUBLE_HYPHEN_DEMERITS: usize = 14;
const FINAL_HYPHEN_DEMERITS: usize = 15;
const ADJ_DEMERITS: usize = 16;
const LOOSENESS: usize = 19;
const TRACING_PARAGRAPHS: usize = 32;
const HANG_AFTER: usize = 41;
const HSIZE: usize = 3;
const HANG_INDENT: usize = 17;
const EMERGENCY_STRETCH: usize = 20;
const LEFT_SKIP: usize = 7;
const RIGHT_SKIP: usize = 8;
const PAR_FILL_SKIP: usize = 14;

const VERY_LOOSE: u8 = 0;
const LOOSE: u8 = 1;
const DECENT: u8 = 2;
const TIGHT: u8 = 3;

type Widths = [Scaled; 7];

#[derive(Clone, Debug)]
enum Item {
    Active {
        fitness: u8,
        hyphenated: bool,
        break_node: Option<usize>,
        line_number: i32,
        total_demerits: i32,
    },
    Delta(Widths),
}

#[derive(Clone, Debug)]
struct Passive {
    cur_break: Option<usize>,
    prev_break: Option<usize>,
    serial: i32,
}

struct Shape {
    last_special_line: i32,
    first_width: Scaled,
    first_indent: Scaled,
    second_width: Scaled,
    second_indent: Scaled,
    par_shape: Option<std::sync::Arc<[i32]>>,
    easy_line: i32,
}

impl Shape {
    fn line(&self, l: i32) -> (Scaled, Scaled) {
        if l > self.last_special_line {
            (self.second_width, self.second_indent)
        } else if let Some(ps) = &self.par_shape {
            let i = 2 * (l as usize - 1);
            (ps[i + 1], ps[i])
        } else {
            (self.first_width, self.first_indent)
        }
    }
}

struct Breaker {
    list: Vec<Node>,
    active: Vec<Item>,
    passive: Vec<Passive>,
    active_width: Widths,
    background: Widths,
    break_width: Widths,
    minimal_demerits: [i32; 4],
    minimum_demerits: i32,
    best_place: [Option<usize>; 4],
    best_pl_line: [i32; 4],
    threshold: i32,
    second_pass: bool,
    final_pass: bool,
    disc_width: Scaled,
    printed_node: Option<usize>,
    pass_number: i32,
    shape: Shape,
    tracing: bool,
    no_shrink_error_yet: bool,
    line_penalty: i32,
    double_hyphen_demerits: i32,
    final_hyphen_demerits: i32,
    adj_demerits: i32,
}

fn char_width(eng: &Eng, n: &Node) -> Scaled {
    match n {
        Node::Char { font, ch } => eng.stomach.fonts.width(*font, *ch),
        Node::Ligature(l) => eng.stomach.fonts.width(l.font, l.ch),
        Node::Box(b) => b.width,
        Node::Rule { width, .. } => *width,
        Node::Kern { width, .. } => *width,
        _ => 0,
    }
}

impl Breaker {
    fn check_shrinkage(&mut self, eng: &mut Eng, g: &mut Glue) -> Res {
        if g.shrink_order != GlueOrder::Normal && g.shrink != 0 {
            if self.no_shrink_error_yet {
                self.no_shrink_error_yet = false;
                if self.tracing {
                    eng.end_diagnostic(true);
                }
                eng.print_err("Infinite glue shrinkage found in a paragraph");
                eng.set_help(&[
                    "The paragraph just ended includes some glue that has",
                    "infinite shrinkability, e.g., `\\hskip 0pt minus 1fil'.",
                    "Such glue doesn't belong there---it allows a paragraph",
                    "of any length to fit on one line. But it's safe to proceed,",
                    "since the offensive shrinkability has been made finite.",
                ]);
                eng.error()?;
                if self.tracing {
                    eng.begin_diagnostic();
                }
            }
            g.shrink_order = GlueOrder::Normal;
        }
        Ok(())
    }

    fn try_break(&mut self, eng: &mut Eng, mut pi: i32, hyphenated: bool, cur_p: Option<usize>) -> Res {
        if pi.abs() >= INF_PENALTY {
            if pi > 0 {
                self.update_printed_node(cur_p);
                return Ok(());
            }
            pi = EJECT_PENALTY;
        }
        let mut no_break_yet = true;
        let mut prev_r: isize = -1;
        let mut prev_prev_r: isize = -1;
        let mut old_l = 0;
        let mut cur_active_width = self.active_width;
        let mut line_width = 0;
        loop {
            let mut r = (prev_r + 1) as usize;
            if let Some(Item::Delta(d)) = self.active.get(r) {
                for k in 1..7 {
                    cur_active_width[k] += d[k];
                }
                prev_prev_r = prev_r;
                prev_r = r as isize;
                continue;
            }
            let l = match self.active.get(r) {
                Some(Item::Active { line_number, .. }) => *line_number,
                _ => MAX_HALFWORD,
            };
            if l > old_l {
                if self.minimum_demerits < AWFUL_BAD && (old_l != self.shape.easy_line || r == self.active.len()) {
                    if no_break_yet {
                        no_break_yet = false;
                        self.compute_break_width(eng, hyphenated, cur_p);
                    }
                    match prev_r {
                        -1 => self.active_width = self.break_width,
                        p => {
                            if let Item::Delta(d) = &mut self.active[p as usize] {
                                for k in 1..7 {
                                    d[k] = d[k] - cur_active_width[k] + self.break_width[k];
                                }
                            } else {
                                let mut d = [0; 7];
                                for k in 1..7 {
                                    d[k] = self.break_width[k] - cur_active_width[k];
                                }
                                self.active.insert(r, Item::Delta(d));
                                prev_prev_r = prev_r;
                                prev_r = r as isize;
                                r += 1;
                            }
                        }
                    }
                    if self.adj_demerits.abs() >= AWFUL_BAD - self.minimum_demerits {
                        self.minimum_demerits = AWFUL_BAD - 1;
                    } else {
                        self.minimum_demerits += self.adj_demerits.abs();
                    }
                    for fit in VERY_LOOSE..=TIGHT {
                        let f = usize::from(fit);
                        if self.minimal_demerits[f] <= self.minimum_demerits {
                            let serial = self.passive.len() as i32 + 1;
                            self.passive.push(Passive {
                                cur_break: cur_p,
                                prev_break: self.best_place[f],
                                serial,
                            });
                            let line_number = self.best_pl_line[f] + 1;
                            let total = self.minimal_demerits[f];
                            self.active.insert(
                                r,
                                Item::Active {
                                    fitness: fit,
                                    hyphenated,
                                    break_node: Some(self.passive.len() - 1),
                                    line_number,
                                    total_demerits: total,
                                },
                            );
                            prev_r = r as isize;
                            r += 1;
                            if self.tracing {
                                eng.print_nl("@@");
                                eng.print_int(serial);
                                eng.print_str(": line ");
                                eng.print_int(line_number - 1);
                                eng.print_char(u32::from('.'));
                                eng.print_int(i32::from(fit));
                                if hyphenated {
                                    eng.print_char(u32::from('-'));
                                }
                                eng.print_str(" t=");
                                eng.print_int(total);
                                eng.print_str(" -> @@");
                                match self.best_place[f] {
                                    None => eng.print_char(u32::from('0')),
                                    Some(p) => eng.print_int(self.passive[p].serial),
                                }
                            }
                        }
                        self.minimal_demerits[f] = AWFUL_BAD;
                    }
                    self.minimum_demerits = AWFUL_BAD;
                    if r != self.active.len() {
                        let mut d = [0; 7];
                        for k in 1..7 {
                            d[k] = cur_active_width[k] - self.break_width[k];
                        }
                        self.active.insert(r, Item::Delta(d));
                        prev_prev_r = prev_r;
                        prev_r = r as isize;
                        r += 1;
                    }
                }
                if r == self.active.len() {
                    self.update_printed_node(cur_p);
                    return Ok(());
                }
                if l > self.shape.easy_line {
                    line_width = self.shape.second_width;
                    old_l = MAX_HALFWORD - 1;
                } else {
                    old_l = l;
                    line_width = self.shape.line(l).0;
                }
            }
            let Item::Active {
                fitness,
                hyphenated: r_hyphenated,
                break_node,
                total_demerits,
                ..
            } = self.active[r].clone()
            else {
                unreachable!("active list item");
            };
            let mut artificial = false;
            let shortfall = line_width - cur_active_width[1];
            let (b, fit_class) = if shortfall > 0 {
                if cur_active_width[3] != 0 || cur_active_width[4] != 0 || cur_active_width[5] != 0 {
                    (0, DECENT)
                } else if shortfall > 7_230_584 && cur_active_width[2] < 1_663_497 {
                    (INF_BAD, VERY_LOOSE)
                } else {
                    let b = badness(shortfall, cur_active_width[2]);
                    let fit = if b > 12 {
                        if b > 99 { VERY_LOOSE } else { LOOSE }
                    } else {
                        DECENT
                    };
                    (b, fit)
                }
            } else {
                let b = if -shortfall > cur_active_width[6] {
                    INF_BAD + 1
                } else {
                    badness(-shortfall, cur_active_width[6])
                };
                (b, if b > 12 { TIGHT } else { DECENT })
            };

            let node_r_stays_active = if b > INF_BAD || pi == EJECT_PENALTY {
                if self.final_pass && self.minimum_demerits == AWFUL_BAD && r + 1 == self.active.len() && prev_r == -1 {
                    artificial = true;
                } else if b > self.threshold {
                    self.deactivate(r, &mut prev_r, &mut prev_prev_r, &mut cur_active_width);
                    continue;
                }
                false
            } else {
                prev_r = r as isize;
                if b > self.threshold {
                    continue;
                }
                true
            };
            let mut d = if artificial {
                0
            } else {
                let mut d = self.line_penalty + b;
                d = if d.abs() >= 10000 { 100_000_000 } else { d * d };
                if pi != 0 {
                    if pi > 0 {
                        d += pi * pi;
                    } else if pi > EJECT_PENALTY {
                        d -= pi * pi;
                    }
                }
                if hyphenated && r_hyphenated {
                    if cur_p.is_some() {
                        d += self.double_hyphen_demerits;
                    } else {
                        d += self.final_hyphen_demerits;
                    }
                }
                if (i32::from(fit_class) - i32::from(fitness)).abs() > 1 {
                    d += self.adj_demerits;
                }
                d
            };
            if self.tracing {
                self.print_feasible(eng, cur_p, break_node, b, pi, d, artificial);
            }
            d += total_demerits;
            let f = usize::from(fit_class);
            if d <= self.minimal_demerits[f] {
                self.minimal_demerits[f] = d;
                self.best_place[f] = break_node;
                self.best_pl_line[f] = l;
                if d < self.minimum_demerits {
                    self.minimum_demerits = d;
                }
            }
            if node_r_stays_active {
                continue;
            }
            self.deactivate(r, &mut prev_r, &mut prev_prev_r, &mut cur_active_width);
        }
    }

    fn deactivate(&mut self, r: usize, prev_r: &mut isize, prev_prev_r: &mut isize, cur: &mut Widths) {
        self.active.remove(r);
        if *prev_r == -1 {
            if let Some(Item::Delta(d)) = self.active.first().cloned() {
                for (a, b) in self.active_width.iter_mut().zip(&d).skip(1) {
                    *a += b;
                }
                *cur = self.active_width;
                self.active.remove(0);
            }
        } else if let Item::Delta(pd) = self.active[*prev_r as usize].clone() {
            let r = *prev_r as usize + 1;
            if r == self.active.len() {
                for k in 1..7 {
                    cur[k] -= pd[k];
                }
                self.active.remove(*prev_r as usize);
                *prev_r = *prev_prev_r;
            } else if let Item::Delta(rd) = self.active[r].clone() {
                for k in 1..7 {
                    cur[k] += rd[k];
                }
                if let Item::Delta(p) = &mut self.active[*prev_r as usize] {
                    for k in 1..7 {
                        p[k] += rd[k];
                    }
                }
                self.active.remove(r);
            }
        }
    }

    fn compute_break_width(&mut self, eng: &Eng, hyphenated: bool, cur_p: Option<usize>) {
        self.break_width = self.background;
        let mut s = cur_p;
        if hyphenated
            && let Some(p) = cur_p
            && let Node::Disc(disc) = &self.list[p]
        {
            let mut v = p;
            for _ in 0..disc.replace_count {
                v += 1;
                self.break_width[1] -= char_width(eng, &self.list[v]);
            }
            for n in &disc.post {
                self.break_width[1] += char_width(eng, n);
            }
            self.break_width[1] += self.disc_width;
            s = if disc.post.is_empty() { Some(v + 1) } else { None };
        }
        while let Some(i) = s {
            match self.list.get(i) {
                Some(Node::Glue(g)) => {
                    self.break_width[1] -= g.spec.width;
                    self.break_width[2 + g.spec.stretch_order as usize] -= g.spec.stretch;
                    self.break_width[6] -= g.spec.shrink;
                }
                Some(Node::Penalty(_)) => {}
                Some(Node::Math { width, .. }) => self.break_width[1] -= width,
                Some(Node::Kern { width, kind }) if *kind == KernKind::Explicit => self.break_width[1] -= width,
                _ => break,
            }
            s = Some(i + 1);
        }
    }

    fn update_printed_node(&mut self, cur_p: Option<usize>) {
        if let (Some(p), Some(printed)) = (cur_p, self.printed_node)
            && p + 1 == printed
            && let Node::Disc(d) = &self.list[p]
        {
            self.printed_node = Some(printed + d.replace_count);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn print_feasible(
        &mut self,
        eng: &mut Eng,
        cur_p: Option<usize>,
        break_node: Option<usize>,
        b: i32,
        pi: i32,
        d: i32,
        artificial: bool,
    ) {
        let start = self.printed_node.unwrap_or(0);
        let end = cur_p.map_or(self.list.len(), |p| p + 1);
        if self.printed_node != Some(end) || cur_p.is_none() && self.printed_node != Some(self.list.len()) {
            if start < end {
                eng.print_nl("");
                if cur_p.is_some() {
                    eng.print_nl("");
                }
                let slice: Vec<Node> = self.list[start..end].to_vec();
                short_display(eng, &slice);
            }
            self.printed_node = Some(end);
        }
        eng.print_nl("@");
        match cur_p.map(|p| &self.list[p]) {
            None => eng.print_esc("par"),
            Some(Node::Glue(_)) => {}
            Some(Node::Penalty(_)) => eng.print_esc("penalty"),
            Some(Node::Disc(_)) => eng.print_esc("discretionary"),
            Some(Node::Kern { .. }) => eng.print_esc("kern"),
            Some(_) => eng.print_esc("math"),
        }
        eng.print_str(" via @@");
        match break_node {
            None => eng.print_char(u32::from('0')),
            Some(p) => eng.print_int(self.passive[p].serial),
        }
        eng.print_str(" b=");
        if b > INF_BAD {
            eng.print_char(u32::from('*'));
        } else {
            eng.print_int(b);
        }
        eng.print_str(" p=");
        eng.print_int(pi);
        eng.print_str(" d=");
        if artificial {
            eng.print_char(u32::from('*'));
        } else {
            eng.print_int(d);
        }
    }
}

fn shape(eng: &Eng) -> Shape {
    let hsize = eng.eqtb().dimen_par(HSIZE);
    let hang_indent = eng.eqtb().dimen_par(HANG_INDENT);
    let hang_after = eng.int_par(HANG_AFTER);
    let par_shape = eng.eqtb().shape(0).clone().filter(|s| !s.is_empty());
    let mut s = Shape {
        last_special_line: 0,
        first_width: 0,
        first_indent: 0,
        second_width: hsize,
        second_indent: 0,
        par_shape: None,
        easy_line: 0,
    };
    if let Some(ps) = par_shape {
        let n = (ps.len() / 2) as i32;
        s.last_special_line = n - 1;
        s.second_width = ps[2 * (n as usize - 1) + 1];
        s.second_indent = ps[2 * (n as usize - 1)];
        s.par_shape = Some(ps);
    } else if hang_indent != 0 {
        s.last_special_line = hang_after.abs();
        if hang_after < 0 {
            s.first_width = hsize - hang_indent.abs();
            s.first_indent = hang_indent.max(0);
            s.second_width = hsize;
            s.second_indent = 0;
        } else {
            s.first_width = hsize;
            s.first_indent = 0;
            s.second_width = hsize - hang_indent.abs();
            s.second_indent = hang_indent.max(0);
        }
    }
    s.easy_line = if eng.int_par(LOOSENESS) == 0 {
        s.last_special_line
    } else {
        MAX_HALFWORD
    };
    s
}

/// `line_break` (§815); `display` selects `\displaywidowpenalty`.
pub(crate) fn line_break(eng: &mut Eng, display: bool) -> Res {
    eng.stomach.pack_begin_line = eng.stomach.mode_line;
    let mut list = std::mem::take(&mut eng.stomach.cur);
    match list.last_mut() {
        Some(last @ Node::Glue(_)) => *last = Node::Penalty(INF_PENALTY),
        _ => list.push(Node::Penalty(INF_PENALTY)),
    }
    list.push(param_glue(eng, PAR_FILL_SKIP));
    let prev_graf_hmode = eng.stomach.prev_graf;
    let init_cur_lang = prev_graf_hmode % 0o200000;
    let init_l_hyf = prev_graf_hmode / 0o20000000;
    let init_r_hyf = (prev_graf_hmode / 0o200000) % 0o100;
    pop_nest(eng);
    let prev_graf = eng.stomach.prev_graf;

    let tracing = eng.int_par(TRACING_PARAGRAPHS) > 0;
    let mut br = Breaker {
        list,
        active: Vec::new(),
        passive: Vec::new(),
        active_width: [0; 7],
        background: [0; 7],
        break_width: [0; 7],
        minimal_demerits: [AWFUL_BAD; 4],
        minimum_demerits: AWFUL_BAD,
        best_place: [None; 4],
        best_pl_line: [0; 4],
        threshold: 0,
        second_pass: false,
        final_pass: false,
        disc_width: 0,
        printed_node: None,
        pass_number: 0,
        shape: shape(eng),
        tracing,
        no_shrink_error_yet: true,
        line_penalty: eng.int_par(LINE_PENALTY),
        double_hyphen_demerits: eng.int_par(DOUBLE_HYPHEN_DEMERITS),
        final_hyphen_demerits: eng.int_par(FINAL_HYPHEN_DEMERITS),
        adj_demerits: eng.int_par(ADJ_DEMERITS),
    };
    let mut left = *eng.eqtb().glue_par(LEFT_SKIP).get();
    let mut right = *eng.eqtb().glue_par(RIGHT_SKIP).get();
    br.check_shrinkage(eng, &mut left)?;
    br.check_shrinkage(eng, &mut right)?;
    br.background[1] = left.width + right.width;
    br.background[2 + left.stretch_order as usize] += left.stretch;
    br.background[2 + right.stretch_order as usize] += right.stretch;
    br.background[6] = left.shrink + right.shrink;

    br.threshold = eng.int_par(PRETOLERANCE);
    if br.threshold >= 0 {
        if tracing {
            eng.begin_diagnostic();
            eng.print_nl("@firstpass");
        }
        br.second_pass = false;
        br.final_pass = false;
    } else {
        br.threshold = eng.int_par(TOLERANCE);
        br.second_pass = true;
        br.final_pass = eng.eqtb().dimen_par(EMERGENCY_STRETCH) <= 0;
        if tracing {
            eng.begin_diagnostic();
        }
    }
    let mut hyph = hyphen::HyphState::default();
    let best_bet = loop {
        if br.threshold > INF_BAD {
            br.threshold = INF_BAD;
        }
        if br.second_pass {
            eng.stomach.hyph.trie_ready = true;
            hyph.cur_lang = init_cur_lang;
            hyph.l_hyf = init_l_hyf;
            hyph.r_hyf = init_r_hyf;
        }
        br.active = vec![Item::Active {
            fitness: DECENT,
            hyphenated: false,
            break_node: None,
            line_number: prev_graf + 1,
            total_demerits: 0,
        }];
        br.active_width = br.background;
        br.passive.clear();
        br.printed_node = Some(0);
        br.pass_number = 0;
        eng.stomach.font_in_short_display = 0;
        let mut cur_p = 0usize;
        let mut auto_breaking = true;
        let mut prev_p = cur_p;
        while cur_p < br.list.len() && !br.active.is_empty() {
            match &br.list[cur_p] {
                Node::Char { .. } => {
                    prev_p = cur_p;
                    while let Some(n @ Node::Char { .. }) = br.list.get(cur_p) {
                        br.active_width[1] += char_width(eng, n);
                        cur_p += 1;
                    }
                    continue;
                }
                Node::Box(b) => br.active_width[1] += b.width,
                Node::Rule { width, .. } => br.active_width[1] += width,
                Node::Whatsit(Whatsit::Language { lang, lhm, rhm }) => {
                    hyph.cur_lang = *lang;
                    hyph.l_hyf = *lhm;
                    hyph.r_hyf = *rhm;
                }
                Node::Whatsit(Whatsit::Backend(item)) => {
                    if let Some((w, _, _)) = item.dimensions(&eng.stomach.backend) {
                        br.active_width[1] += w;
                    }
                }
                Node::Glue(_) => {
                    if auto_breaking {
                        let p = &br.list[prev_p];
                        let legal = p.is_char()
                            || p.precedes_break()
                            || matches!(p, Node::Kern { kind, .. } if *kind != KernKind::Explicit);
                        if legal {
                            br.try_break(eng, 0, false, Some(cur_p))?;
                        }
                    }
                    let mut g = match &br.list[cur_p] {
                        Node::Glue(g) => g.spec,
                        _ => unreachable!("glue node"),
                    };
                    br.check_shrinkage(eng, &mut g)?;
                    if let Node::Glue(gn) = &mut br.list[cur_p] {
                        gn.spec = g;
                    }
                    br.active_width[1] += g.width;
                    br.active_width[2 + g.stretch_order as usize] += g.stretch;
                    br.active_width[6] += g.shrink;
                    if br.second_pass && auto_breaking {
                        hyphen::try_hyphenate(eng, &mut br.list, cur_p, &mut hyph)?;
                    }
                }
                Node::Kern { width, kind } => {
                    let width = *width;
                    if *kind == KernKind::Explicit {
                        br.kern_break(eng, cur_p, auto_breaking, width)?;
                    } else {
                        br.active_width[1] += width;
                    }
                }
                Node::Ligature(l) => br.active_width[1] += eng.stomach.fonts.width(l.font, l.ch),
                Node::Disc(d) => {
                    let pre_width: Scaled = d.pre.iter().map(|n| char_width(eng, n)).sum();
                    let pre_empty = d.pre.is_empty();
                    let replace = d.replace_count;
                    br.disc_width = 0;
                    if pre_empty {
                        br.try_break(eng, eng.int_par(EX_HYPHEN_PENALTY), true, Some(cur_p))?;
                    } else {
                        br.disc_width = pre_width;
                        br.active_width[1] += pre_width;
                        br.try_break(eng, eng.int_par(HYPHEN_PENALTY), true, Some(cur_p))?;
                        br.active_width[1] -= pre_width;
                    }
                    let mut s = cur_p + 1;
                    for _ in 0..replace {
                        br.active_width[1] += char_width(eng, &br.list[s]);
                        s += 1;
                    }
                    prev_p = cur_p;
                    cur_p = s;
                    continue;
                }
                Node::Math { width, edge } => {
                    let width = *width;
                    auto_breaking = *edge == MathEdge::After;
                    br.kern_break(eng, cur_p, auto_breaking, width)?;
                }
                Node::Penalty(p) => {
                    let p = *p;
                    br.try_break(eng, p, false, Some(cur_p))?;
                }
                _ => {}
            }
            prev_p = cur_p;
            cur_p += 1;
        }
        if cur_p >= br.list.len() {
            br.try_break(eng, EJECT_PENALTY, true, None)?;
            if !br.active.is_empty() {
                let mut best = None;
                let mut fewest = AWFUL_BAD;
                for (i, item) in br.active.iter().enumerate() {
                    if let Item::Active { total_demerits, .. } = item
                        && *total_demerits < fewest
                    {
                        fewest = *total_demerits;
                        best = Some(i);
                    }
                }
                let best = best.expect("an active node");
                let looseness = eng.int_par(LOOSENESS);
                if looseness == 0 {
                    break best;
                }
                let best_line = active_line(&br.active[best]);
                let mut best_bet = best;
                let mut actual = 0;
                for (i, item) in br.active.iter().enumerate() {
                    if let Item::Active {
                        line_number,
                        total_demerits,
                        ..
                    } = item
                    {
                        let diff = line_number - best_line;
                        if (diff < actual && looseness <= diff) || (diff > actual && looseness >= diff) {
                            best_bet = i;
                            actual = diff;
                            fewest = *total_demerits;
                        } else if diff == actual && *total_demerits < fewest {
                            best_bet = i;
                            fewest = *total_demerits;
                        }
                    }
                }
                if actual == looseness || br.final_pass {
                    break best_bet;
                }
            }
        }
        if !br.second_pass {
            if tracing {
                eng.print_nl("@secondpass");
            }
            br.threshold = eng.int_par(TOLERANCE);
            br.second_pass = true;
            br.final_pass = eng.eqtb().dimen_par(EMERGENCY_STRETCH) <= 0;
        } else {
            if tracing {
                eng.print_nl("@emergencypass");
            }
            br.background[2] += eng.eqtb().dimen_par(EMERGENCY_STRETCH);
            br.final_pass = true;
        }
    };
    if tracing {
        eng.end_diagnostic(true);
        eng.normalize_selector();
    }
    post_line_break(eng, br, best_bet, display)?;
    eng.stomach.pack_begin_line = 0;
    Ok(())
}

fn active_line(item: &Item) -> i32 {
    match item {
        Item::Active { line_number, .. } => *line_number,
        Item::Delta(_) => 0,
    }
}

impl Breaker {
    fn kern_break(&mut self, eng: &mut Eng, cur_p: usize, auto_breaking: bool, width: Scaled) -> Res {
        if auto_breaking && matches!(self.list.get(cur_p + 1), Some(Node::Glue(_))) {
            self.try_break(eng, 0, false, Some(cur_p))?;
        }
        self.active_width[1] += width;
        Ok(())
    }
}

fn penalty_from(eng: &Eng, i: usize, n: i32) -> Option<i32> {
    let s = eng.eqtb().shape(i).clone()?;
    let count = *s.first()?;
    let r = n.min(count).max(0) as usize;
    s.get(r).copied()
}

/// `post_line_break` (§877).
fn post_line_break(eng: &mut Eng, br: Breaker, best_bet: usize, display: bool) -> Res {
    let Item::Active {
        break_node,
        line_number: best_line,
        ..
    } = br.active[best_bet].clone()
    else {
        return Ok(());
    };
    let mut breaks = Vec::new();
    let mut q = break_node;
    while let Some(p) = q {
        breaks.push(br.passive[p].cur_break);
        q = br.passive[p].prev_break;
    }
    breaks.reverse();
    let shape = br.shape;
    let mut list: Vec<Option<Node>> = br.list.into_iter().map(Some).collect();
    let mut start = 0usize;
    let mut carry: Vec<Node> = Vec::new();
    let prev_graf = eng.stomach.prev_graf;
    for (cur_line, (bi, brk)) in (prev_graf + 1..).zip(breaks.iter().enumerate()) {
        let mut line: Vec<Node> = std::mem::take(&mut carry);
        let mut disc_break = false;
        let mut post_disc_break = false;
        let end = brk.unwrap_or(list.len());
        for slot in list.iter_mut().take(end).skip(start) {
            if let Some(n) = slot.take() {
                line.push(n);
            }
        }
        let mut next_start = end + 1;
        let right = param_glue(eng, RIGHT_SKIP);
        match brk.map(|b| list[b].take()) {
            Some(Some(Node::Glue(_))) => line.push(right),
            Some(Some(Node::Disc(d))) => {
                for slot in list.iter_mut().skip(end + 1).take(d.replace_count) {
                    slot.take();
                }
                next_start = end + 1 + d.replace_count;
                let d = *d;
                if !d.post.is_empty() {
                    carry = d.post;
                    post_disc_break = true;
                }
                line.push(Node::Disc(Box::new(super::node::DiscNode {
                    pre: Vec::new(),
                    post: Vec::new(),
                    replace_count: 0,
                })));
                line.extend(d.pre);
                disc_break = true;
                line.push(right);
            }
            Some(Some(mut other)) => {
                match &mut other {
                    Node::Math { width, .. } | Node::Kern { width, .. } => *width = 0,
                    _ => {}
                }
                line.push(other);
                line.push(right);
            }
            _ => line.push(right),
        }
        if eng.eqtb().glue_par(LEFT_SKIP).get() != &Glue::default()
            || !std::sync::Arc::ptr_eq(&eng.eqtb().glue_par(LEFT_SKIP).0, &eng.eqtb().zero_glue().0)
        {
            let lg = *eng.eqtb().glue_par(LEFT_SKIP).get();
            if !(lg.width == 0 && lg.stretch == 0 && lg.shrink == 0) {
                line.insert(0, param_glue(eng, LEFT_SKIP));
            }
        }
        let (cur_width, cur_indent) = shape.line(cur_line);
        eng.stomach.adjust = Some(Vec::new());
        let mut just = hpack(eng, line, Spec::Exactly(cur_width));
        just.shift = cur_indent;
        if display && cur_line + 1 == best_line {
            eng.stomach.just_box = Some(just.clone());
        }
        append_to_vlist(eng, just);
        if let Some(adjust) = eng.stomach.adjust.take() {
            eng.stomach.cur.extend(adjust);
        }
        if cur_line + 1 != best_line {
            let mut pen = penalty_from(eng, 1, cur_line).unwrap_or_else(|| eng.int_par(INTER_LINE_PENALTY));
            pen += match penalty_from(eng, 2, cur_line - prev_graf) {
                Some(p) => p,
                None if cur_line == prev_graf + 1 => eng.int_par(CLUB_PENALTY),
                None => 0,
            };
            let widow_array = if display { 4 } else { 3 };
            pen += match penalty_from(eng, widow_array, best_line - cur_line - 1) {
                Some(p) => p,
                None if cur_line + 2 == best_line => {
                    eng.int_par(if display { DISPLAY_WIDOW_PENALTY } else { WIDOW_PENALTY })
                }
                None => 0,
            };
            if disc_break {
                pen += eng.int_par(BROKEN_PENALTY);
            }
            if pen != 0 {
                eng.stomach.cur.push(Node::Penalty(pen));
            }
        }
        start = next_start;
        if let Some(next) = breaks.get(bi + 1)
            && !post_disc_break
        {
            let stop = next.unwrap_or(list.len());
            while start < stop {
                let Some(n) = &list[start] else {
                    start += 1;
                    continue;
                };
                let discardable = matches!(n, Node::Glue(_) | Node::Penalty(_) | Node::Math { .. })
                    || matches!(n, Node::Kern { kind, .. } if *kind == KernKind::Explicit);
                if !discardable {
                    break;
                }
                list[start] = None;
                start += 1;
            }
        }
    }
    eng.stomach.prev_graf = best_line - 1;
    Ok(())
}

/// Whether protrusion skips `n` at a margin (pdfTeX's `cp_skipable`).
pub(crate) fn cp_skipable(eng_state: &super::backend::BackendState, n: &Node) -> bool {
    match n {
        Node::Ins(_) | Node::Mark(_) | Node::Adjust(_) | Node::Penalty(_) => true,
        Node::Whatsit(super::node::Whatsit::Backend(item)) => item.dimensions(eng_state).is_none(),
        Node::Whatsit(_) => true,
        Node::Disc(d) => d.pre.is_empty() && d.post.is_empty() && d.replace_count == 0,
        Node::Math { width: 0, .. } => true,
        Node::Kern { width, kind } => *width == 0 || *kind == KernKind::Normal,
        Node::Glue(g) => g.zero_ptr,
        Node::Box(b) => {
            b.dir == super::node::BoxDir::H && b.width == 0 && b.height == 0 && b.depth == 0 && b.list.is_empty()
        }
        _ => false,
    }
}

/// The margin kern at the left or right end of box `b`, as `\leftmarginkern` and `\rightmarginkern` report it.
pub(crate) fn margin_kern_of(state: &super::backend::BackendState, b: &super::node::BoxNode, left: bool) -> Scaled {
    let skip = |n: &Node, param: usize| {
        cp_skipable(state, n) || matches!(n, Node::Glue(g) if g.kind == GlueKind::Param(param as u8))
    };
    let found = if left {
        b.list.iter().find(|n| !skip(n, LEFT_SKIP))
    } else {
        b.list.iter().rev().find(|n| !skip(n, RIGHT_SKIP))
    };
    match found {
        Some(Node::MarginKern { width, left: l }) if *l == left => *width,
        _ => 0,
    }
}

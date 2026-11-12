//! Displaying boxes and lists: `show_box`, `short_display` and `show_activities` (§173–§198, §218).

use rtex_core::params::GLUE_PARAMS;
use rtex_core::{GlueOrder, Token};

use super::node::{
    BoxDir, Delimiter, FractionNoad, GlueKind, GlueSign, KernKind, Limits, MathEdge, MathField, Noad, NoadKind, Node,
    RUNNING, Whatsit,
};
use super::page::PageContents;
use super::{Eng, IGNORE_DEPTH, MMODE, VMODE, print_font_id};
use crate::units::{Scaled, UNITY};

const SHOW_BOX_BREADTH: usize = 24;
const SHOW_BOX_DEPTH: usize = 25;

struct Display {
    prefix: Vec<u32>,
    depth_threshold: usize,
    breadth_max: usize,
}

/// `show_box` (§198).
pub(crate) fn show_box(eng: &mut Eng, list: &[Node]) {
    let depth = eng.int_par(SHOW_BOX_DEPTH).max(0) as usize;
    let breadth = eng.int_par(SHOW_BOX_BREADTH);
    let mut d = Display {
        prefix: Vec::new(),
        depth_threshold: depth,
        breadth_max: if breadth <= 0 { 5 } else { breadth as usize },
    };
    d.show_list(eng, list);
    eng.print_ln();
}

/// `show_box` applied to a single node, as `\showbox` and box diagnostics call it.
pub(crate) fn show_box_node(eng: &mut Eng, node: &Node) {
    show_box(eng, std::slice::from_ref(node));
}

fn print_rule_dimen(eng: &mut Eng, d: Scaled) {
    if d == RUNNING {
        eng.print_char(u32::from('*'));
    } else {
        eng.print_scaled(d);
    }
}

fn print_glue_order(eng: &mut Eng, d: Scaled, order: GlueOrder, unit: &str) {
    eng.print_glue(d, order, unit);
}

pub(crate) fn print_mark(eng: &mut Eng, toks: &[Token]) {
    eng.print_char(u32::from('{'));
    let limit = eng.config().max_print_line.saturating_sub(10);
    eng.show_token_list(toks, None, limit);
    eng.print_char(u32::from('}'));
}

fn print_font_and_char(eng: &mut Eng, font: usize, ch: u32) {
    print_font_id(eng, font);
    eng.print_char(u32::from(' '));
    eng.print(ch);
}

impl Display {
    fn show_list(&mut self, eng: &mut Eng, list: &[Node]) {
        if self.prefix.len() > self.depth_threshold {
            if !list.is_empty() {
                eng.print_str(" []");
            }
            return;
        }
        for (n, node) in list.iter().enumerate() {
            eng.print_ln();
            eng.slow_print(&self.prefix.clone());
            if n + 1 > self.breadth_max {
                eng.print_str("etc.");
                return;
            }
            self.show_node(eng, node);
        }
    }

    fn nested(&mut self, eng: &mut Eng, c: char, list: &[Node]) {
        self.prefix.push(u32::from(c));
        self.show_list(eng, list);
        self.prefix.pop();
    }

    fn show_node(&mut self, eng: &mut Eng, node: &Node) {
        match node {
            Node::Char { font, ch } => print_font_and_char(eng, *font, *ch),
            Node::Box(b) => {
                eng.print_esc(if b.dir == BoxDir::H { "h" } else { "v" });
                eng.print_str("box(");
                eng.print_scaled(b.height);
                eng.print_char(u32::from('+'));
                eng.print_scaled(b.depth);
                eng.print_str(")x");
                eng.print_scaled(b.width);
                let g = b.glue_set;
                if g != 0.0 && b.glue_sign != GlueSign::Normal {
                    eng.print_str(", glue set ");
                    if b.glue_sign == GlueSign::Shrinking {
                        eng.print_str("- ");
                    }
                    if g.abs() > 20000.0 {
                        if g > 0.0 {
                            eng.print_char(u32::from('>'));
                        } else {
                            eng.print_str("< -");
                        }
                        print_glue_order(eng, 20000 * UNITY, b.glue_order, "");
                    } else {
                        print_glue_order(eng, (f64::from(UNITY) * g).round() as Scaled, b.glue_order, "");
                    }
                }
                if b.shift != 0 {
                    eng.print_str(", shifted ");
                    eng.print_scaled(b.shift);
                }
                if b.display && b.dir == BoxDir::H && eng.etex_ex() {
                    eng.print_str(", display");
                }
                self.nested(eng, '.', &b.list);
            }
            Node::Unset(u) => {
                eng.print_esc("unset");
                eng.print_str("box(");
                eng.print_scaled(u.height);
                eng.print_char(u32::from('+'));
                eng.print_scaled(u.depth);
                eng.print_str(")x");
                eng.print_scaled(u.width);
                if u.span_count != 0 {
                    eng.print_str(" (");
                    eng.print_int(i32::from(u.span_count) + 1);
                    eng.print_str(" columns)");
                }
                if u.stretch != 0 {
                    eng.print_str(", stretch ");
                    print_glue_order(eng, u.stretch, u.stretch_order, "");
                }
                if u.shrink != 0 {
                    eng.print_str(", shrink ");
                    print_glue_order(eng, u.shrink, u.shrink_order, "");
                }
                self.nested(eng, '.', &u.list);
            }
            Node::Rule { width, depth, height } => {
                eng.print_esc("rule(");
                print_rule_dimen(eng, *height);
                eng.print_char(u32::from('+'));
                print_rule_dimen(eng, *depth);
                eng.print_str(")x");
                print_rule_dimen(eng, *width);
            }
            Node::Ins(ins) => {
                eng.print_esc("insert");
                eng.print_int(i32::from(ins.number));
                eng.print_str(", natural size ");
                eng.print_scaled(ins.height);
                eng.print_str("; split(");
                eng.print_spec(&ins.split_top, "");
                eng.print_char(u32::from(','));
                eng.print_scaled(ins.depth);
                eng.print_str("); float cost ");
                eng.print_int(ins.float_cost);
                self.nested(eng, '.', &ins.list);
            }
            Node::Whatsit(w) => show_whatsit(eng, w),
            Node::Glue(g) => {
                if g.kind.is_leaders() {
                    eng.print_esc("");
                    match g.kind {
                        GlueKind::CLeaders => eng.print_char(u32::from('c')),
                        GlueKind::XLeaders => eng.print_char(u32::from('x')),
                        _ => {}
                    }
                    eng.print_str("leaders ");
                    eng.print_spec(&g.spec, "");
                    let leader: Vec<Node> = g.leader.iter().map(|b| (**b).clone()).collect();
                    self.nested(eng, '.', &leader);
                } else {
                    eng.print_esc("glue");
                    match g.kind {
                        GlueKind::Normal => {}
                        GlueKind::Param(i) => {
                            eng.print_char(u32::from('('));
                            eng.print_esc(GLUE_PARAMS.get(usize::from(i)).copied().unwrap_or("?"));
                            eng.print_char(u32::from(')'));
                        }
                        GlueKind::CondMath => {
                            eng.print_char(u32::from('('));
                            eng.print_esc("nonscript");
                            eng.print_char(u32::from(')'));
                        }
                        GlueKind::Mu => {
                            eng.print_char(u32::from('('));
                            eng.print_esc("mskip");
                            eng.print_char(u32::from(')'));
                        }
                        _ => {}
                    }
                    if g.kind != GlueKind::CondMath {
                        eng.print_char(u32::from(' '));
                        let unit = if g.kind == GlueKind::Mu { "mu" } else { "" };
                        eng.print_spec(&g.spec, unit);
                    }
                }
            }
            Node::Kern { width, kind } => {
                if *kind == KernKind::Mu {
                    eng.print_esc("mkern");
                    eng.print_scaled(*width);
                    eng.print_str("mu");
                } else {
                    eng.print_esc("kern");
                    if *kind != KernKind::Normal {
                        eng.print_char(u32::from(' '));
                    }
                    eng.print_scaled(*width);
                    if *kind == KernKind::Accent {
                        eng.print_str(" (for accent)");
                    }
                }
            }
            Node::Math { width, edge } => {
                eng.print_esc("math");
                eng.print_str(if *edge == MathEdge::Before { "on" } else { "off" });
                if *width != 0 {
                    eng.print_str(", surrounded ");
                    eng.print_scaled(*width);
                }
            }
            Node::Ligature(l) => {
                print_font_and_char(eng, l.font, l.ch);
                eng.print_str(" (ligature ");
                if l.left_hit {
                    eng.print_char(u32::from('|'));
                }
                eng.stomach.font_in_short_display = l.font;
                for &c in &l.original {
                    eng.print(c);
                }
                if l.right_hit {
                    eng.print_char(u32::from('|'));
                }
                eng.print_char(u32::from(')'));
            }
            Node::Penalty(p) => {
                eng.print_esc("penalty ");
                eng.print_int(*p);
            }
            Node::Disc(d) => {
                eng.print_esc("discretionary");
                if d.replace_count > 0 {
                    eng.print_str(" replacing ");
                    eng.print_int(d.replace_count as i32);
                }
                self.nested(eng, '.', &d.pre);
                self.nested(eng, '|', &d.post);
            }
            Node::Mark(m) => {
                eng.print_esc("mark");
                if m.class != 0 {
                    eng.print_char(u32::from('s'));
                    eng.print_int(m.class);
                }
                print_mark(eng, &m.toks);
            }
            Node::Adjust(list) => {
                eng.print_esc("vadjust");
                self.nested(eng, '.', list);
            }
            Node::MarginKern { width, left } => {
                eng.print_esc("kern");
                eng.print_scaled(*width);
                eng.print_str(if *left { " (left margin)" } else { " (right margin)" });
            }
            Node::Style(style) => print_style(eng, i32::from(*style)),
            Node::Choice(lists) => {
                eng.print_esc("mathchoice");
                for (c, list) in ['D', 'T', 'S', 's'].into_iter().zip(lists.iter()) {
                    self.nested(eng, c, list);
                }
            }
            Node::Noad(noad) => self.show_noad(eng, noad),
            Node::Fraction(f) => self.show_fraction(eng, f),
        }
    }

    fn show_noad(&mut self, eng: &mut Eng, noad: &Noad) {
        match &noad.kind {
            NoadKind::Ord => eng.print_esc("mathord"),
            NoadKind::Op => eng.print_esc("mathop"),
            NoadKind::Bin => eng.print_esc("mathbin"),
            NoadKind::Rel => eng.print_esc("mathrel"),
            NoadKind::Open => eng.print_esc("mathopen"),
            NoadKind::Close => eng.print_esc("mathclose"),
            NoadKind::Punct => eng.print_esc("mathpunct"),
            NoadKind::Inner => eng.print_esc("mathinner"),
            NoadKind::Over => eng.print_esc("overline"),
            NoadKind::Under => eng.print_esc("underline"),
            NoadKind::VCenter => eng.print_esc("vcenter"),
            NoadKind::Radical(d) => {
                eng.print_esc("radical");
                print_delimiter(eng, d);
            }
            NoadKind::Accent { fam, ch } => {
                eng.print_esc("accent");
                print_fam_and_char(eng, *fam, *ch);
            }
            NoadKind::Left(d) => {
                eng.print_esc("left");
                print_delimiter(eng, d);
            }
            NoadKind::Right { delim, middle } => {
                eng.print_esc(if *middle { "middle" } else { "right" });
                print_delimiter(eng, delim);
            }
        }
        if noad.scripts_allowed() {
            match noad.limits {
                Limits::Normal => {}
                Limits::Limits => eng.print_esc("limits"),
                Limits::NoLimits => eng.print_esc("nolimits"),
            }
            self.subsidiary(eng, &noad.nucleus, '.');
        }
        self.subsidiary(eng, &noad.supscr, '^');
        self.subsidiary(eng, &noad.subscr, '_');
    }

    fn show_fraction(&mut self, eng: &mut Eng, f: &FractionNoad) {
        eng.print_esc("fraction, thickness ");
        match f.thickness {
            None => eng.print_str("= default"),
            Some(t) => eng.print_scaled(t),
        }
        if !f.left.is_null() {
            eng.print_str(", left-delimiter ");
            print_delimiter(eng, &f.left);
        }
        if !f.right.is_null() {
            eng.print_str(", right-delimiter ");
            print_delimiter(eng, &f.right);
        }
        self.subsidiary(eng, &f.numerator, '\\');
        self.subsidiary(eng, &f.denominator, '/');
    }

    /// `print_subsidiary_data` (§692).
    fn subsidiary(&mut self, eng: &mut Eng, field: &MathField, c: char) {
        if self.prefix.len() >= self.depth_threshold {
            if *field != MathField::Empty {
                eng.print_str(" []");
            }
            return;
        }
        self.prefix.push(u32::from(c));
        match field {
            MathField::Char { fam, ch } | MathField::TextChar { fam, ch } => {
                eng.print_ln();
                eng.slow_print(&self.prefix.clone());
                print_fam_and_char(eng, *fam, *ch);
            }
            MathField::SubBox(b) => self.show_list(eng, &[Node::Box(b.clone())]),
            MathField::SubMlist(list) if list.is_empty() => {
                eng.print_ln();
                eng.slow_print(&self.prefix.clone());
                eng.print_str("{}");
            }
            MathField::SubMlist(list) => self.show_list(eng, list),
            MathField::Empty => {}
        }
        self.prefix.pop();
    }
}

/// `print_fam_and_char` (§691).
pub(crate) fn print_fam_and_char(eng: &mut Eng, fam: u8, ch: u32) {
    eng.print_esc("fam");
    eng.print_int(i32::from(fam));
    eng.print_char(u32::from(' '));
    eng.print(ch);
}

/// `print_delimiter` (§691).
pub(crate) fn print_delimiter(eng: &mut Eng, d: &Delimiter) {
    let a = (i32::from(d.small_fam) * 256 + d.small_char as i32) * 0x1000
        + i32::from(d.large_fam) * 256
        + d.large_char as i32;
    if a < 0 {
        eng.print_int(a);
    } else {
        eng.print_hex(a);
    }
}

/// `print_style` (§694).
pub(crate) fn print_style(eng: &mut Eng, c: i32) {
    match c / 2 {
        0 => eng.print_esc("displaystyle"),
        1 => eng.print_esc("textstyle"),
        2 => eng.print_esc("scriptstyle"),
        3 => eng.print_esc("scriptscriptstyle"),
        _ => eng.print_str("Unknown style!"),
    }
}

fn print_write_whatsit(eng: &mut Eng, name: &str, stream: u8) {
    eng.print_esc(name);
    match stream {
        0..=15 => eng.print_int(i32::from(stream)),
        16 => eng.print_char(u32::from('*')),
        _ => eng.print_char(u32::from('-')),
    }
}

fn show_whatsit(eng: &mut Eng, w: &Whatsit) {
    match w {
        Whatsit::Open { stream, name } => {
            print_write_whatsit(eng, "openout", *stream);
            eng.print_char(u32::from('='));
            eng.print_str(name);
        }
        Whatsit::Write { stream, toks } => {
            print_write_whatsit(eng, "write", *stream);
            print_mark(eng, toks);
        }
        Whatsit::Close { stream } => print_write_whatsit(eng, "closeout", *stream),
        Whatsit::Special(toks) => {
            eng.print_esc("special");
            print_mark(eng, toks);
        }
        Whatsit::Language { lang, lhm, rhm } => {
            eng.print_esc("setlanguage");
            eng.print_int(*lang);
            eng.print_str(" (hyphenmin ");
            eng.print_int(*lhm);
            eng.print_char(u32::from(','));
            eng.print_int(*rhm);
            eng.print_char(u32::from(')'));
        }
        Whatsit::Backend(item) => item.show(eng),
    }
}

/// `short_display` (§174).
pub(crate) fn short_display(eng: &mut Eng, list: &[Node]) {
    let mut i = 0;
    while i < list.len() {
        match &list[i] {
            Node::Char { font, ch } => {
                if *font != eng.stomach.font_in_short_display {
                    print_font_id(eng, *font);
                    eng.print_char(u32::from(' '));
                    eng.stomach.font_in_short_display = *font;
                }
                eng.print(*ch);
            }
            Node::Box(_) | Node::Ins(_) | Node::Whatsit(_) | Node::Mark(_) | Node::Adjust(_) | Node::Unset(_) => {
                eng.print_str("[]")
            }
            Node::Rule { .. } => eng.print_char(u32::from('|')),
            Node::Glue(g) => {
                if !g.zero_ptr {
                    eng.print_char(u32::from(' '));
                }
            }
            Node::Math { .. } => eng.print_char(u32::from('$')),
            Node::Ligature(l) => {
                if l.font != eng.stomach.font_in_short_display {
                    print_font_id(eng, l.font);
                    eng.print_char(u32::from(' '));
                    eng.stomach.font_in_short_display = l.font;
                }
                for &c in &l.original {
                    eng.print(c);
                }
            }
            Node::Disc(d) => {
                short_display(eng, &d.pre);
                short_display(eng, &d.post);
                i += d.replace_count;
            }
            _ => {}
        }
        i += 1;
    }
}

/// `show_activities` (§218), for `\showlists`.
pub(crate) fn show_activities(eng: &mut Eng) {
    eng.print_nl("");
    eng.print_ln();
    let depth = eng.stomach.nest.len();
    for p in (0..=depth).rev() {
        let (mode, mode_line, prev_graf, prev_depth, space_factor, clang, list) = if p == depth {
            (
                eng.mode,
                eng.stomach.mode_line,
                eng.stomach.prev_graf,
                eng.aux.prev_depth,
                eng.aux.space_factor,
                eng.stomach.clang,
                eng.stomach.cur.clone(),
            )
        } else {
            let s = &eng.stomach.nest[p];
            (
                s.mode,
                s.mode_line,
                s.prev_graf,
                s.prev_depth,
                s.space_factor,
                s.clang,
                s.list.clone(),
            )
        };
        eng.print_nl("### ");
        eng.print_mode(mode);
        eng.print_str(" entered at line ");
        eng.print_int(mode_line.abs());
        if mode == super::HMODE && prev_graf != 0o40600000 {
            eng.print_str(" (language");
            eng.print_int(prev_graf % 0o200000);
            eng.print_str(":hyphenmin");
            eng.print_int(prev_graf / 0o20000000);
            eng.print_char(u32::from(','));
            eng.print_int((prev_graf / 0o200000) % 0o100);
            eng.print_char(u32::from(')'));
        }
        if mode_line < 0 {
            eng.print_str(" (\\output routine)");
        }
        if p == 0 {
            show_page_status(eng);
            if !list.is_empty() {
                eng.print_nl("### recent contributions:");
            }
        }
        show_box(eng, &list);
        match mode.abs() {
            m if m == VMODE => {
                eng.print_nl("prevdepth ");
                if prev_depth <= IGNORE_DEPTH {
                    eng.print_str("ignored");
                } else {
                    eng.print_scaled(prev_depth);
                }
                if prev_graf != 0 {
                    eng.print_str(", prevgraf ");
                    eng.print_int(prev_graf);
                    eng.print_str(" line");
                    if prev_graf != 1 {
                        eng.print_char(u32::from('s'));
                    }
                }
            }
            m if m == super::HMODE => {
                eng.print_nl("spacefactor ");
                eng.print_int(space_factor);
                if mode > 0 && clang > 0 {
                    eng.print_str(", current language ");
                    eng.print_int(clang);
                }
            }
            m if m == MMODE => super::math::show_incompleat(eng, p),
            _ => {}
        }
    }
}

fn show_page_status(eng: &mut Eng) {
    if eng.stomach.page.list.is_empty() {
        return;
    }
    eng.print_nl("### current page:");
    if eng.stomach.page.output_active {
        eng.print_str(" (held over for next output)");
    }
    let page = eng.stomach.page.list.clone();
    show_box(eng, &page);
    if eng.stomach.page.contents != PageContents::Empty {
        eng.print_nl("total height ");
        super::page::print_totals(eng);
        eng.print_nl(" goal height ");
        eng.print_scaled(eng.aux.page_so_far[0]);
        super::page::show_page_inserts(eng);
    }
}

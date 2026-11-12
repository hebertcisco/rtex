//! Building math lists (§1136–§1206, with e-TeX's `\middle`).

use rtex_core::cmd;
use rtex_core::{GroupKind, Loc, Res, Value};

use super::build::{end_graf, normal_paragraph, scan_spec};
use super::mlist::{DISPLAY_STYLE, TEXT_SIZE, TEXT_STYLE, check_fonts, math_quad, mlist_to_hlist};
use super::node::{
    BoxNode, Delimiter, FractionNoad, GlueKind, GlueNode, GlueSign, INF_PENALTY, Limits, MathEdge, MathField, Noad,
    NoadKind, Node,
};
use super::pack::{NATURAL, append_to_vlist, hpack, param_glue, vpack};
use super::show::show_box_node;
use super::{Eng, GroupData, HMODE, IGNORE_DEPTH, MMODE, Spec, VMODE, pop_nest, push_nest, tail_append};
use crate::units::Scaled;

const PRE_DISPLAY_PENALTY: usize = 11;
const POST_DISPLAY_PENALTY: usize = 12;
const HANG_AFTER: usize = 41;
const LEFT_HYPHEN_MIN: usize = 51;
const RIGHT_HYPHEN_MIN: usize = 52;
const PRE_DISPLAY_DIRECTION: usize = 60;
const MATH_SURROUND: usize = 1;
const HSIZE: usize = 3;
const PRE_DISPLAY_SIZE: usize = 13;
const DISPLAY_WIDTH: usize = 14;
const DISPLAY_INDENT: usize = 15;
const HANG_INDENT: usize = 17;
const ABOVE_DISPLAY_SKIP: usize = 3;
const BELOW_DISPLAY_SKIP: usize = 4;
const ABOVE_DISPLAY_SHORT_SKIP: usize = 5;
const BELOW_DISPLAY_SHORT_SKIP: usize = 6;
const EVERY_MATH: usize = 2;
const EVERY_DISPLAY: usize = 3;
const EVERY_VBOX: usize = 5;
const CUR_FAM: usize = 44;
const VAR_CODE: i32 = 0x7000;
const MAX_DIMEN: Scaled = super::node::MAX_DIMEN;

const LEFT_NOAD: u32 = 30;
const RIGHT_NOAD: u32 = 31;
const MIDDLE_NOAD: u32 = 1;
const ABOVE_CODE: u32 = 0;
const OVER_CODE: u32 = 1;
const DELIMITED_CODE: u32 = 3;

/// The noad field a subformula fills when its group ends (TeX's `saved(0)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MathTarget {
    Nucleus,
    Supscr,
    Subscr,
}

fn field_of(noad: &mut Noad, target: MathTarget) -> &mut MathField {
    match target {
        MathTarget::Nucleus => &mut noad.nucleus,
        MathTarget::Supscr => &mut noad.supscr,
        MathTarget::Subscr => &mut noad.subscr,
    }
}

fn tail_noad(eng: &mut Eng) -> Option<&mut Noad> {
    match eng.stomach.cur.last_mut() {
        Some(Node::Noad(q)) => Some(q),
        _ => None,
    }
}

fn set_target(eng: &mut Eng, target: MathTarget, value: MathField) {
    if let Some(q) = tail_noad(eng) {
        *field_of(q, target) = value;
    }
}

fn fam_in_range(eng: &Eng) -> Option<u8> {
    let f = eng.int_par(CUR_FAM);
    (0..16).contains(&f).then_some(f as u8)
}

fn math_char_field(eng: &Eng, c: i32) -> MathField {
    let fam = match fam_in_range(eng) {
        Some(f) if c >= VAR_CODE => f,
        _ => ((c / 256) % 16) as u8,
    };
    MathField::Char {
        fam,
        ch: (c % 256) as u32,
    }
}

/// `push_math` (§1136).
fn push_math(eng: &mut Eng, group: GroupKind) -> Res {
    push_nest(eng);
    eng.mode = -MMODE;
    eng.stomach.incompleat = None;
    eng.new_save_level(group)
}

fn go_into_ordinary_math(eng: &mut Eng) -> Res {
    push_math(eng, GroupKind::MathShift)?;
    eng.eq_define(Loc::IntPar(CUR_FAM as u16), Value::Int(-1), false)?;
    eng.begin_toks_param(EVERY_MATH)
}

/// `init_math` (§1138).
pub(crate) fn init_math(eng: &mut Eng) -> Res {
    eng.get_token()?;
    if eng.cur_cmd() == cmd::MATH_SHIFT && eng.mode > 0 {
        go_into_display_math(eng)
    } else {
        eng.back_input()?;
        go_into_ordinary_math(eng)
    }
}

/// Enters display math (§1145, with e-TeX's prototype box and direction).
fn go_into_display_math(eng: &mut Eng) -> Res {
    let mut w = -MAX_DIMEN;
    if eng.stomach.cur.is_empty() {
        pop_nest(eng);
    } else {
        eng.stomach.just_box = None;
        super::linebreak::line_break(eng, true)?;
        if let Some(just) = eng.stomach.just_box.take() {
            w = natural_width(eng, &just);
        }
    }
    let (l, s) = display_line(eng);
    push_math(eng, GroupKind::MathShift)?;
    eng.mode = MMODE;
    eng.eq_define(Loc::IntPar(CUR_FAM as u16), Value::Int(-1), false)?;
    eng.eq_define(Loc::DimenPar(PRE_DISPLAY_SIZE as u16), Value::Int(w), false)?;
    if eng.etex_ex() {
        eng.eq_define(Loc::IntPar(PRE_DISPLAY_DIRECTION as u16), Value::Int(0), false)?;
    }
    eng.eq_define(Loc::DimenPar(DISPLAY_WIDTH as u16), Value::Int(l), false)?;
    eng.eq_define(Loc::DimenPar(DISPLAY_INDENT as u16), Value::Int(s), false)?;
    eng.begin_toks_param(EVERY_DISPLAY)?;
    if eng.stomach.nest.len() == 1 {
        super::page::build_page(eng)?;
    }
    Ok(())
}

/// The natural width of the line before a display, plus two ems (§1146).
fn natural_width(eng: &Eng, just: &BoxNode) -> Scaled {
    let quad = super::fonts::param(eng, eng.eqtb().cur_font().max(0) as usize, 5);
    let mut v = just.shift + 2 * quad;
    let mut w = -MAX_DIMEN;
    for p in &just.list {
        let (d, visible) = match p {
            Node::Char { font, ch } => (eng.stomach.fonts.width(*font, *ch), true),
            Node::Ligature(l) => (eng.stomach.fonts.width(l.font, l.ch), true),
            Node::Box(b) => (b.width, true),
            Node::Rule { width, .. } => (*width, true),
            Node::Kern { width, .. } | Node::Math { width, .. } | Node::MarginKern { width, .. } => (*width, false),
            Node::Whatsit(_) => (eng.stomach.backend.node_size(p).map_or(0, |s| s.0), false),
            Node::Glue(g) => {
                let q = &g.spec;
                match just.glue_sign {
                    GlueSign::Stretching if just.glue_order == q.stretch_order && q.stretch != 0 => v = MAX_DIMEN,
                    GlueSign::Shrinking if just.glue_order == q.shrink_order && q.shrink != 0 => v = MAX_DIMEN,
                    _ => {}
                }
                (q.width, g.kind.is_leaders())
            }
            _ => (0, false),
        };
        if visible {
            if v < MAX_DIMEN {
                v += d;
                w = v;
            } else {
                return MAX_DIMEN;
            }
        } else if v < MAX_DIMEN {
            v += d;
        }
    }
    w
}

/// The length and shift of the display lines (§1149).
fn display_line(eng: &Eng) -> (Scaled, Scaled) {
    let prev_graf = eng.stomach.prev_graf;
    match eng.eqtb().shape(0).as_ref().filter(|s| s.len() >= 2) {
        None => {
            let hang_indent = eng.eqtb().dimen_par(HANG_INDENT);
            let hang_after = eng.int_par(HANG_AFTER);
            let hsize = eng.eqtb().dimen_par(HSIZE);
            if hang_indent != 0 && ((hang_after >= 0 && prev_graf + 2 > hang_after) || (prev_graf + 1 < -hang_after)) {
                (hsize - hang_indent.abs(), hang_indent.max(0))
            } else {
                (hsize, 0)
            }
        }
        Some(ps) => {
            let n = (ps.len() / 2) as i32;
            let k = if prev_graf + 2 >= n { n } else { prev_graf + 2 };
            let i = 2 * (k as usize - 1);
            (ps[i + 1], ps[i])
        }
    }
}

/// Whether the command may be used in non-inner modes (`privileged`, §1051).
fn privileged(eng: &mut Eng) -> Res<bool> {
    if eng.mode > 0 {
        Ok(true)
    } else {
        eng.report_illegal_case()?;
        Ok(false)
    }
}

/// `start_eq_no` (§1142).
fn start_eq_no(eng: &mut Eng) -> Res {
    eng.stomach.groups.push(GroupData::EqNo {
        leqno: eng.cur_chr() == 1,
    });
    go_into_ordinary_math(eng)
}

/// Treats `cur_chr` as an active character (§1152).
fn treat_as_active(eng: &mut Eng, c: u32) -> Res {
    let t = eng.active_token(c);
    eng.back_list(vec![t])
}

/// `scan_math` (§1151): fills `target` of the tail noad, now or when a subformula ends.
fn scan_math(eng: &mut Eng, target: MathTarget) -> Res {
    loop {
        eng.get_next_nonblank_nonrelax_x()?;
        let mut cmd = eng.cur_cmd();
        let mut ch = eng.cur_chr();
        let c = loop {
            match cmd {
                cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN => break eng.eqtb().math_code(ch) as i32,
                cmd::CHAR_NUM => {
                    ch = eng.scan_char_num()? as u32;
                    cmd = cmd::CHAR_GIVEN;
                }
                cmd::MATH_CHAR_NUM => break eng.scan_fifteen_bit_int()?,
                cmd::MATH_GIVEN => break ch as i32,
                cmd::DELIM_NUM => break eng.scan_twenty_seven_bit_int()? / 0x1000,
                _ => {
                    eng.back_input()?;
                    eng.scan_left_brace()?;
                    eng.stomach.groups.push(GroupData::MathField { target });
                    return push_math(eng, GroupKind::Math);
                }
            }
        };
        if c == 0x8000 {
            treat_as_active(eng, ch)?;
            continue;
        }
        let field = math_char_field(eng, c);
        set_target(eng, target, field);
        return Ok(());
    }
}

/// `set_math_char` (§1155).
fn set_math_char(eng: &mut Eng, c: i32, ch: u32) -> Res {
    if c >= 0x8000 {
        return treat_as_active(eng, ch);
    }
    let kind = if c >= VAR_CODE {
        NoadKind::Ord
    } else {
        match c / 0x1000 {
            1 => NoadKind::Op,
            2 => NoadKind::Bin,
            3 => NoadKind::Rel,
            4 => NoadKind::Open,
            5 => NoadKind::Close,
            6 => NoadKind::Punct,
            _ => NoadKind::Ord,
        }
    };
    let mut p = Noad::new(kind);
    p.nucleus = math_char_field(eng, c);
    if c < VAR_CODE
        && let MathField::Char { fam, .. } = &mut p.nucleus
    {
        *fam = ((c / 256) % 16) as u8;
    }
    tail_append(eng, Node::Noad(Box::new(p)));
    Ok(())
}

fn comp_kind(c: u32) -> NoadKind {
    match c {
        17 => NoadKind::Op,
        18 => NoadKind::Bin,
        19 => NoadKind::Rel,
        20 => NoadKind::Open,
        21 => NoadKind::Close,
        22 => NoadKind::Punct,
        23 => NoadKind::Inner,
        26 => NoadKind::Under,
        27 => NoadKind::Over,
        _ => NoadKind::Ord,
    }
}

/// `math_limit_switch` (§1159).
fn math_limit_switch(eng: &mut Eng) -> Res {
    let c = eng.cur_chr();
    if let Some(q) = tail_noad(eng)
        && q.kind == NoadKind::Op
    {
        q.limits = match c {
            1 => Limits::Limits,
            2 => Limits::NoLimits,
            _ => Limits::Normal,
        };
        return Ok(());
    }
    eng.print_err("Limit controls must follow a math operator");
    eng.set_help(&["I'm ignoring this misplaced \\limits or \\nolimits command."]);
    eng.error()
}

/// `scan_delimiter` (§1160).
fn scan_delimiter(eng: &mut Eng, r: bool) -> Res<Delimiter> {
    let mut v = if r {
        eng.scan_twenty_seven_bit_int()?
    } else {
        eng.get_next_nonblank_nonrelax_x()?;
        match eng.cur_cmd() {
            cmd::LETTER | cmd::OTHER_CHAR => eng.eqtb().del_code(eng.cur_chr()),
            cmd::DELIM_NUM => eng.scan_twenty_seven_bit_int()?,
            _ => -1,
        }
    };
    if v < 0 {
        eng.print_err("Missing delimiter (. inserted)");
        eng.set_help(&[
            "I was expecting to see something like `(' or `\\{' or",
            "`\\}' here. If you typed, e.g., `{' instead of `\\{', you",
            "should probably delete the `{' by typing `1' now, so that",
            "braces don't get unbalanced. Otherwise just proceed.",
            "Acceptable delimiters are characters whose \\delcode is",
            "nonnegative, or you can use `\\delimiter <delimiter code>'.",
        ]);
        eng.back_error()?;
        v = 0;
    }
    Ok(Delimiter {
        small_fam: ((v / 0o4000000) % 16) as u8,
        small_char: ((v / 0o10000) % 256) as u32,
        large_fam: ((v / 256) % 16) as u8,
        large_char: (v % 256) as u32,
    })
}

/// `math_radical` (§1163).
fn math_radical(eng: &mut Eng) -> Res {
    tail_append(
        eng,
        Node::Noad(Box::new(Noad::new(NoadKind::Radical(Delimiter::default())))),
    );
    let d = scan_delimiter(eng, true)?;
    if let Some(q) = tail_noad(eng) {
        q.kind = NoadKind::Radical(d);
    }
    scan_math(eng, MathTarget::Nucleus)
}

/// `math_ac` (§1165).
fn math_ac(eng: &mut Eng) -> Res {
    if eng.cur_cmd() == cmd::ACCENT {
        eng.print_err("Please use ");
        eng.print_esc("mathaccent");
        eng.print_str(" for accents in math mode");
        eng.set_help(&[
            "I'm changing \\accent to \\mathaccent here; wish me luck.",
            "(Accents are not the same in formulas as they are in text.)",
        ]);
        eng.error()?;
    }
    tail_append(eng, Node::Noad(Box::new(Noad::new(NoadKind::Accent { fam: 0, ch: 0 }))));
    let v = eng.scan_fifteen_bit_int()?;
    let fam = match fam_in_range(eng) {
        Some(f) if v >= VAR_CODE => f,
        _ => ((v / 256) % 16) as u8,
    };
    if let Some(q) = tail_noad(eng) {
        q.kind = NoadKind::Accent {
            fam,
            ch: (v % 256) as u32,
        };
    }
    scan_math(eng, MathTarget::Nucleus)
}

/// `mmode+vcenter` (§1167).
fn begin_vcenter(eng: &mut Eng) -> Res {
    let spec = scan_spec(eng)?;
    eng.new_save_level(GroupKind::VCenter)?;
    eng.stomach.groups.push(GroupData::VCenter { spec });
    eng.scan_left_brace()?;
    normal_paragraph(eng)?;
    push_nest(eng);
    eng.mode = -VMODE;
    eng.aux.prev_depth = IGNORE_DEPTH;
    eng.begin_toks_param(EVERY_VBOX)
}

/// `append_choices` (§1172).
fn append_choices(eng: &mut Eng) -> Res {
    tail_append(eng, Node::Choice(Box::default()));
    eng.stomach.groups.push(GroupData::Choice { part: 0 });
    push_math(eng, GroupKind::MathChoice)?;
    eng.scan_left_brace()
}

/// `build_choices` (§1174).
fn build_choices(eng: &mut Eng) -> Res {
    eng.unsave()?;
    let p = fin_mlist(eng, None)?;
    let Some(GroupData::Choice { part }) = eng.stomach.groups.pop() else {
        return Ok(());
    };
    if let Some(Node::Choice(lists)) = eng.stomach.cur.last_mut() {
        lists[usize::from(part)] = p;
    }
    if part == 3 {
        return Ok(());
    }
    eng.stomach.groups.push(GroupData::Choice { part: part + 1 });
    push_math(eng, GroupKind::MathChoice)?;
    eng.scan_left_brace()
}

/// `sub_sup` (§1176).
fn sub_sup(eng: &mut Eng) -> Res {
    let target = if eng.cur_cmd() == cmd::SUP_MARK {
        MathTarget::Supscr
    } else {
        MathTarget::Subscr
    };
    let mut occupied = None;
    if let Some(q) = tail_noad(eng)
        && q.scripts_allowed()
    {
        occupied = Some(*field_of(q, target) != MathField::Empty);
    }
    if occupied != Some(false) {
        tail_append(eng, Node::Noad(Box::new(Noad::new(NoadKind::Ord))));
        if occupied == Some(true) {
            if target == MathTarget::Supscr {
                eng.print_err("Double superscript");
                eng.set_help(&["I treat `x^1^2' essentially like `x^1{}^2'."]);
            } else {
                eng.print_err("Double subscript");
                eng.set_help(&["I treat `x_1_2' essentially like `x_1{}_2'."]);
            }
            eng.error()?;
        }
    }
    scan_math(eng, target)
}

/// `math_fraction` (§1181).
fn math_fraction(eng: &mut Eng) -> Res {
    let c = eng.cur_chr();
    if eng.stomach.incompleat.is_some() {
        if c >= DELIMITED_CODE {
            scan_delimiter(eng, false)?;
            scan_delimiter(eng, false)?;
        }
        if c % DELIMITED_CODE == ABOVE_CODE {
            eng.scan_normal_dimen()?;
        }
        eng.print_err("Ambiguous; you need another { and }");
        eng.set_help(&[
            "I'm ignoring this fraction specification, since I don't",
            "know whether a construction like `x \\over y \\over z'",
            "means `{x \\over y} \\over z' or `x \\over {y \\over z}'.",
        ]);
        return eng.error();
    }
    let numerator = std::mem::take(&mut eng.stomach.cur);
    let mut f = FractionNoad {
        thickness: None,
        numerator: MathField::SubMlist(numerator),
        denominator: MathField::Empty,
        left: Delimiter::default(),
        right: Delimiter::default(),
        new_hlist: None,
    };
    if c >= DELIMITED_CODE {
        f.left = scan_delimiter(eng, false)?;
        f.right = scan_delimiter(eng, false)?;
    }
    f.thickness = match c % DELIMITED_CODE {
        ABOVE_CODE => Some(eng.scan_normal_dimen()?),
        OVER_CODE => None,
        _ => Some(0),
    };
    eng.stomach.incompleat = Some(Box::new(f));
    Ok(())
}

/// `fin_mlist` (§1184): finishes the current mlist, appending `p`, and pops the nest.
fn fin_mlist(eng: &mut Eng, p: Option<Node>) -> Res<Vec<Node>> {
    let list = std::mem::take(&mut eng.stomach.cur);
    let q = match eng.stomach.incompleat.take() {
        Some(mut f) => {
            f.denominator = MathField::SubMlist(list);
            match p {
                None => vec![Node::Fraction(f)],
                Some(p) => {
                    let MathField::SubMlist(num) = std::mem::take(&mut f.numerator) else {
                        unreachable!()
                    };
                    let left_first = matches!(num.first(), Some(Node::Noad(n)) if matches!(n.kind, NoadKind::Left(_)));
                    let Some(d) = eng.stomach.delim_ptr.filter(|_| left_first) else {
                        eng.confusion("right")?;
                        return Ok(Vec::new());
                    };
                    let mut num = num;
                    let rest = num.split_off(d + 1);
                    f.numerator = MathField::SubMlist(rest);
                    num.push(Node::Fraction(f));
                    num.push(p);
                    num
                }
            }
        }
        None => {
            let mut list = list;
            list.extend(p);
            list
        }
    };
    pop_nest(eng);
    Ok(q)
}

/// The right brace of a `math_group` (§1186).
fn end_math_group(eng: &mut Eng) -> Res {
    eng.unsave()?;
    let Some(GroupData::MathField { target }) = eng.stomach.groups.pop() else {
        return Ok(());
    };
    let p = fin_mlist(eng, None)?;
    if let [Node::Noad(n)] = p.as_slice() {
        if n.kind == NoadKind::Ord && n.subscr == MathField::Empty && n.supscr == MathField::Empty {
            let Some(Node::Noad(n)) = p.into_iter().next() else {
                unreachable!()
            };
            set_target(eng, target, n.nucleus);
            return Ok(());
        }
        if matches!(n.kind, NoadKind::Accent { .. })
            && target == MathTarget::Nucleus
            && matches!(eng.stomach.cur.last(), Some(Node::Noad(t)) if t.kind == NoadKind::Ord)
        {
            let accent = p.into_iter().next();
            eng.stomach.cur.pop();
            eng.stomach.cur.extend(accent);
            return Ok(());
        }
    }
    set_target(eng, target, MathField::SubMlist(p));
    Ok(())
}

/// `math_left_right` (§1191, with e-TeX's `\middle`).
fn math_left_right(eng: &mut Eng) -> Res {
    let t = eng.cur_chr();
    if t != LEFT_NOAD && eng.cur_group() != GroupKind::MathLeft {
        if eng.cur_group() == GroupKind::MathShift {
            scan_delimiter(eng, false)?;
            eng.print_err("Extra ");
            if t == MIDDLE_NOAD {
                eng.print_esc("middle");
                eng.set_help(&["I'm ignoring a \\middle that had no matching \\left."]);
            } else {
                eng.print_esc("right");
                eng.set_help(&["I'm ignoring a \\right that had no matching \\left."]);
            }
            return eng.error();
        }
        return eng.off_save();
    }
    let d = scan_delimiter(eng, false)?;
    let kind = match t {
        LEFT_NOAD => NoadKind::Left(d),
        _ => NoadKind::Right {
            delim: d,
            middle: t == MIDDLE_NOAD,
        },
    };
    let p = Node::Noad(Box::new(Noad::new(kind)));
    let q = if t == LEFT_NOAD {
        vec![p]
    } else {
        let q = fin_mlist(eng, Some(p))?;
        eng.unsave()?;
        q
    };
    if t != RIGHT_NOAD {
        push_math(eng, GroupKind::MathLeft)?;
        eng.stomach.cur = q;
        eng.stomach.delim_ptr = Some(eng.stomach.cur.len() - 1);
    } else {
        let mut inner = Noad::new(NoadKind::Inner);
        inner.nucleus = MathField::SubMlist(q);
        tail_append(eng, Node::Noad(Box::new(inner)));
    }
    Ok(())
}

/// Checks that another `$` follows (§1197).
fn check_dollar(eng: &mut Eng) -> Res {
    eng.get_x_token()?;
    if eng.cur_cmd() != cmd::MATH_SHIFT {
        eng.print_err("Display math should end with $$");
        eng.set_help(&[
            "The `$' that I just saw supposedly matches a previous `$$'.",
            "So I shall assume that you typed `$$' both times.",
        ]);
        eng.back_error()?;
    }
    Ok(())
}

fn fonts_ok(eng: &mut Eng) -> Res<bool> {
    if check_fonts(eng)? {
        Ok(true)
    } else {
        flush_math(eng);
        Ok(false)
    }
}

/// `after_math` (§1194).
fn after_math(eng: &mut Eng) -> Res {
    let mut danger = !fonts_ok(eng)?;
    let mut m = eng.mode;
    let mut l = false;
    let mut p = fin_mlist(eng, None)?;
    let mut a = None;
    if eng.mode == -m {
        check_dollar(eng)?;
        let hl = mlist_to_hlist(eng, p, TEXT_STYLE, false)?;
        let mut eqno = hpack(eng, hl, NATURAL);
        eqno.display = true;
        a = Some(eqno);
        eng.unsave()?;
        if let Some(GroupData::EqNo { leqno }) = eng.stomach.groups.pop() {
            l = leqno;
        }
        danger = !fonts_ok(eng)?;
        m = eng.mode;
        p = fin_mlist(eng, None)?;
    }
    if m < 0 {
        let surround = eng.eqtb().dimen_par(MATH_SURROUND);
        tail_append(
            eng,
            Node::Math {
                width: surround,
                edge: MathEdge::Before,
            },
        );
        let penalties = eng.mode > 0;
        let hl = mlist_to_hlist(eng, p, TEXT_STYLE, penalties)?;
        eng.stomach.cur.extend(hl);
        tail_append(
            eng,
            Node::Math {
                width: surround,
                edge: MathEdge::After,
            },
        );
        eng.aux.space_factor = 1000;
        eng.unsave()
    } else {
        if a.is_none() {
            check_dollar(eng)?;
        }
        finish_displayed_math(eng, p, a, l, danger)
    }
}

/// Finishes displayed math (§1199–§1205).
fn finish_displayed_math(eng: &mut Eng, p: Vec<Node>, a: Option<BoxNode>, l: bool, danger: bool) -> Res {
    let hl = mlist_to_hlist(eng, p, DISPLAY_STYLE, false)?;
    eng.stomach.adjust = Some(Vec::new());
    let mut b = hpack(eng, hl, NATURAL);
    let t = eng.stomach.adjust.take().unwrap_or_default();
    let mut w = b.width;
    let z = eng.eqtb().dimen_par(DISPLAY_WIDTH);
    let mut s = eng.eqtb().dimen_par(DISPLAY_INDENT);
    if eng.int_par(PRE_DISPLAY_DIRECTION) < 0 {
        s = -s - z;
    }
    let (mut e, q) = match &a {
        Some(a) if !danger => (a.width, a.width + math_quad(eng, TEXT_SIZE)),
        _ => (0, 0),
    };
    if w + q > z {
        let shrink = eng.stomach.pack_totals.1;
        if e != 0 && (w - shrink[0] + q <= z || shrink[1] != 0 || shrink[2] != 0 || shrink[3] != 0) {
            let list = std::mem::take(&mut b.list);
            b = hpack(eng, list, Spec::Exactly(z - q));
        } else {
            e = 0;
            if w > z {
                let list = std::mem::take(&mut b.list);
                b = hpack(eng, list, Spec::Exactly(z));
            }
        }
        w = b.width;
    }
    b.display = true;
    let mut d = rtex_core::half(z - w);
    if e > 0 && d < 2 * e {
        d = rtex_core::half(z - w - e);
        if matches!(b.list.first(), Some(Node::Glue(_))) {
            d = 0;
        }
    }
    let pre_penalty = eng.int_par(PRE_DISPLAY_PENALTY);
    tail_append(eng, Node::Penalty(pre_penalty));
    let (g1, mut g2) = if d + s <= eng.eqtb().dimen_par(PRE_DISPLAY_SIZE) || l {
        (ABOVE_DISPLAY_SKIP, Some(BELOW_DISPLAY_SKIP))
    } else {
        (ABOVE_DISPLAY_SHORT_SKIP, Some(BELOW_DISPLAY_SHORT_SKIP))
    };
    let mut a = a;
    if l && e == 0 {
        if let Some(a) = a.take() {
            app_display(eng, a, 0);
        }
        tail_append(eng, Node::Penalty(INF_PENALTY));
    } else {
        let g = param_glue(eng, g1);
        tail_append(eng, g);
    }
    if e != 0 {
        let r = Node::kern(z - w - e - d, super::node::KernKind::Normal);
        let a_box = a.take().map(|x| Node::Box(Box::new(x)));
        let list = if l {
            d = 0;
            a_box.into_iter().chain([r, Node::Box(Box::new(b))]).collect()
        } else {
            [Node::Box(Box::new(b)), r].into_iter().chain(a_box).collect()
        };
        b = hpack(eng, list, NATURAL);
    }
    app_display(eng, b, d);
    if let Some(a) = a.filter(|_| e == 0 && !l) {
        tail_append(eng, Node::Penalty(INF_PENALTY));
        let shift = z - a.width;
        app_display(eng, a, shift);
        g2 = None;
    }
    eng.stomach.cur.extend(t);
    let post_penalty = eng.int_par(POST_DISPLAY_PENALTY);
    tail_append(eng, Node::Penalty(post_penalty));
    if let Some(g2) = g2 {
        let g = param_glue(eng, g2);
        tail_append(eng, g);
    }
    resume_after_display(eng)
}

/// `app_display` (e-TeX) when `\predisplaydirection` is zero: the box shifted by the indent.
fn app_display(eng: &mut Eng, mut b: BoxNode, d: Scaled) {
    b.shift = eng.eqtb().dimen_par(DISPLAY_INDENT) + d;
    append_to_vlist(eng, b);
}

/// `resume_after_display` (§1200).
fn resume_after_display(eng: &mut Eng) -> Res {
    if eng.cur_group() != GroupKind::MathShift {
        return eng.confusion("display");
    }
    eng.unsave()?;
    eng.stomach.prev_graf += 3;
    push_nest(eng);
    eng.mode = HMODE;
    eng.aux.space_factor = 1000;
    let lang = super::build::cur_lang(eng);
    eng.stomach.clang = lang;
    let lhm = eng.int_par(LEFT_HYPHEN_MIN).clamp(1, 63);
    let rhm = eng.int_par(RIGHT_HYPHEN_MIN).clamp(1, 63);
    eng.stomach.prev_graf = (lhm * 0o100 + rhm) * 0o200000 + lang;
    eng.get_x_token()?;
    if eng.cur_cmd() != cmd::SPACER {
        eng.back_input()?;
    }
    if eng.stomach.nest.len() == 1 {
        super::page::build_page(eng)?;
    }
    Ok(())
}

/// A box or indentation appended in math mode becomes a noad (§1076, §1093).
pub(crate) fn append_box_noad(eng: &mut Eng, node: Node) {
    let mut p = Noad::new(NoadKind::Ord);
    if let Node::Box(b) = node {
        p.nucleus = MathField::SubBox(b);
    }
    tail_append(eng, Node::Noad(Box::new(p)));
}

/// The incompleat noad of nest level `level` in `\showlists` (§219).
pub(crate) fn show_incompleat(eng: &mut Eng, level: usize) {
    let f = if level == eng.stomach.nest.len() {
        eng.stomach.incompleat.clone()
    } else {
        eng.stomach.nest[level].incompleat.clone()
    };
    if let Some(f) = f {
        eng.print_str("this will begin denominator of:");
        show_box_node(eng, &Node::Fraction(f));
    }
}

/// `mmode+halign` (§1130).
pub(crate) fn halign_in_display(eng: &mut Eng) -> Res {
    if privileged(eng)? {
        if eng.cur_group() == GroupKind::MathShift {
            super::align::init_align(eng)
        } else {
            eng.off_save()
        }
    } else {
        Ok(())
    }
}

/// The math-mode cases of `main_control` (§1045).
pub(crate) fn command(eng: &mut Eng) -> Res {
    let c = eng.cur_chr();
    match eng.cur_cmd() {
        cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN => {
            let code = eng.eqtb().math_code(c) as i32;
            set_math_char(eng, code, c)
        }
        cmd::CHAR_NUM => {
            let ch = eng.scan_char_num()? as u32;
            let code = eng.eqtb().math_code(ch) as i32;
            set_math_char(eng, code, ch)
        }
        cmd::MATH_CHAR_NUM => {
            let v = eng.scan_fifteen_bit_int()?;
            set_math_char(eng, v, c)
        }
        cmd::MATH_GIVEN => set_math_char(eng, c as i32, c),
        cmd::DELIM_NUM => {
            let v = eng.scan_twenty_seven_bit_int()?;
            set_math_char(eng, v / 0x1000, c)
        }
        cmd::LEFT_BRACE => {
            tail_append(eng, Node::Noad(Box::new(Noad::new(NoadKind::Ord))));
            eng.back_input()?;
            scan_math(eng, MathTarget::Nucleus)
        }
        cmd::MATH_COMP => {
            tail_append(eng, Node::Noad(Box::new(Noad::new(comp_kind(c)))));
            scan_math(eng, MathTarget::Nucleus)
        }
        cmd::LIMIT_SWITCH => math_limit_switch(eng),
        cmd::RADICAL => math_radical(eng),
        cmd::ACCENT | cmd::MATH_ACCENT => math_ac(eng),
        cmd::VCENTER => begin_vcenter(eng),
        cmd::MATH_STYLE => {
            tail_append(eng, Node::Style(c as u8));
            Ok(())
        }
        cmd::NON_SCRIPT => {
            let mut g = GlueNode::new(rtex_core::Glue::default());
            g.kind = GlueKind::CondMath;
            tail_append(eng, Node::Glue(g));
            Ok(())
        }
        cmd::MATH_CHOICE => append_choices(eng),
        cmd::SUB_MARK | cmd::SUP_MARK => sub_sup(eng),
        cmd::ABOVE => math_fraction(eng),
        cmd::LEFT_RIGHT => math_left_right(eng),
        cmd::MATH_SHIFT => {
            if eng.cur_group() == GroupKind::MathShift {
                after_math(eng)
            } else {
                eng.off_save()
            }
        }
        cmd::EQ_NO => {
            if privileged(eng)? {
                if eng.cur_group() == GroupKind::MathShift {
                    start_eq_no(eng)
                } else {
                    eng.off_save()
                }
            } else {
                Ok(())
            }
        }
        _ => eng.report_illegal_case(),
    }
}

/// Right braces of the groups math mode opens (§1085, §1168, §1173, §1186).
pub(crate) fn end_group(eng: &mut Eng, kind: GroupKind) -> Res {
    match kind {
        GroupKind::Math => end_math_group(eng),
        GroupKind::MathChoice => build_choices(eng),
        GroupKind::VCenter => {
            end_graf(eng)?;
            eng.unsave()?;
            let Some(GroupData::VCenter { spec }) = eng.stomach.groups.pop() else {
                return Ok(());
            };
            let list = std::mem::take(&mut eng.stomach.cur);
            let p = vpack(eng, list, spec);
            pop_nest(eng);
            let mut n = Noad::new(NoadKind::VCenter);
            n.nucleus = MathField::SubBox(Box::new(p));
            tail_append(eng, Node::Noad(Box::new(n)));
            Ok(())
        }
        _ => eng.unsave(),
    }
}

/// Whether a display has material before an `\halign` (§1145's "Improper \halign" check).
pub(crate) fn display_has_material(eng: &Eng) -> bool {
    !eng.stomach.cur.is_empty() || eng.stomach.incompleat.is_some()
}

/// `flush_math` (§718).
pub(crate) fn flush_math(eng: &mut Eng) {
    eng.stomach.cur.clear();
    eng.stomach.incompleat = None;
}

/// Finishes an alignment in a display (§1206).
pub(crate) fn finish_display_alignment(eng: &mut Eng, list: Vec<Node>, aux: (Scaled, i32)) -> Res {
    eng.do_assignments()?;
    if eng.cur_cmd() != cmd::MATH_SHIFT {
        eng.print_err("Missing $$ inserted");
        eng.set_help(&[
            "Displays can use special alignments (like \\eqalignno)",
            "only if nothing but the alignment itself is between $$'s.",
        ]);
        eng.back_error()?;
    } else {
        check_dollar(eng)?;
    }
    pop_nest(eng);
    let pre = eng.int_par(PRE_DISPLAY_PENALTY);
    tail_append(eng, Node::Penalty(pre));
    let g = param_glue(eng, ABOVE_DISPLAY_SKIP);
    tail_append(eng, g);
    eng.stomach.cur.extend(list);
    let post = eng.int_par(POST_DISPLAY_PENALTY);
    tail_append(eng, Node::Penalty(post));
    let g = param_glue(eng, BELOW_DISPLAY_SKIP);
    tail_append(eng, g);
    eng.aux.prev_depth = aux.0;
    resume_after_display(eng)
}

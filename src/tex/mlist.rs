//! Typesetting math formulas: `mlist_to_hlist` and its subroutines (§699–§767).

use rtex_core::{Glue, GlueOrder, Res, half, nx_plus_y, x_over_n, xn_over_d};

use super::Eng;
use super::Spec;
use super::build::fil_glue;
use super::fonts::{char_warning, new_character, param};
use super::node::{
    BoxDir, BoxNode, Delimiter, GlueKind, GlueNode, INF_PENALTY, KernKind, Limits, MathField, Noad, NoadKind, Node,
    RUNNING,
};
use super::pack::{NATURAL, hpack, vpack};
use crate::fonts::tfm::Tag;
use crate::units::Scaled;

pub(crate) const DISPLAY_STYLE: u8 = 0;
pub(crate) const TEXT_STYLE: u8 = 2;
pub(crate) const SCRIPT_STYLE: u8 = 4;
pub(crate) const CRAMPED: u8 = 1;

pub(crate) const TEXT_SIZE: u16 = 0;
pub(crate) const SCRIPT_SIZE: u16 = 16;
pub(crate) const SCRIPT_SCRIPT_SIZE: u16 = 32;

pub(crate) const TOTAL_MATHSY_PARAMS: usize = 22;
pub(crate) const TOTAL_MATHEX_PARAMS: usize = 13;

const BIN_OP_PENALTY: usize = 9;
const REL_PENALTY: usize = 10;
const DELIMITER_FACTOR: usize = 18;
const DELIMITER_SHORTFALL: usize = 10;
const NULL_DELIMITER_SPACE: usize = 11;
const SCRIPT_SPACE: usize = 12;
const THIN_MU_SKIP: u8 = 15;
const MED_MU_SKIP: u8 = 16;
const THICK_MU_SKIP: u8 = 17;

const MATH_SPACING: &[u8; 64] = b"02340001\
22*40001\
33**3**3\
44*04004\
00*00000\
02340001\
11*11111\
12341011";

pub(crate) fn cramped_style(s: u8) -> u8 {
    2 * (s / 2) + CRAMPED
}

pub(crate) fn sub_style(s: u8) -> u8 {
    2 * (s / 4) + SCRIPT_STYLE + CRAMPED
}

pub(crate) fn sup_style(s: u8) -> u8 {
    2 * (s / 4) + SCRIPT_STYLE + (s % 2)
}

fn num_style(s: u8) -> u8 {
    s + 2 - 2 * (s / 6)
}

fn denom_style(s: u8) -> u8 {
    2 * (s / 2) + CRAMPED + 2 - 2 * (s / 6)
}

/// The font of family `fam` at `size` (`fam_fnt(fam+size)`).
pub(crate) fn fam_fnt(eng: &Eng, fam: u16, size: u16) -> usize {
    eng.eqtb().math_font(size / 16, fam).max(0) as usize
}

/// `mathsy(k)` at `size`.
pub(crate) fn mathsy(eng: &Eng, k: usize, size: u16) -> Scaled {
    param(eng, fam_fnt(eng, 2, size), k - 1)
}

fn mathex(eng: &Eng, k: usize, size: u16) -> Scaled {
    param(eng, fam_fnt(eng, 3, size), k - 1)
}

pub(crate) fn math_quad(eng: &Eng, size: u16) -> Scaled {
    mathsy(eng, 6, size)
}

pub(crate) fn axis_height(eng: &Eng, size: u16) -> Scaled {
    mathsy(eng, 22, size)
}

/// The style-dependent state of `mlist_to_hlist`: `cur_style`, `cur_size` and `cur_mu`.
#[derive(Clone, Copy)]
struct Cx {
    style: u8,
    size: u16,
    mu: Scaled,
}

impl Cx {
    fn new(eng: &Eng, style: u8) -> Self {
        let mut cx = Cx { style, size: 0, mu: 0 };
        cx.set_style(eng, style);
        cx
    }

    fn set_style(&mut self, eng: &Eng, style: u8) {
        self.style = style;
        self.size = if style < SCRIPT_STYLE {
            TEXT_SIZE
        } else {
            16 * u16::from((style - TEXT_STYLE) / 2)
        };
        self.mu = x_over_n(math_quad(eng, self.size), 18).quotient;
    }

    fn rule_thickness(&self, eng: &Eng) -> Scaled {
        mathex(eng, 8, self.size)
    }

    fn sy(&self, eng: &Eng, k: usize) -> Scaled {
        mathsy(eng, k, self.size)
    }

    fn ex(&self, eng: &Eng, k: usize) -> Scaled {
        mathex(eng, k, self.size)
    }
}

fn fraction_rule(t: Scaled) -> Node {
    Node::rule(RUNNING, t, 0)
}

fn kern(w: Scaled) -> Node {
    Node::kern(w, KernKind::Normal)
}

fn boxed(b: BoxNode) -> Node {
    Node::Box(Box::new(b))
}

/// `overbar` (§705).
fn overbar(eng: &mut Eng, b: BoxNode, k: Scaled, t: Scaled) -> BoxNode {
    vpack(eng, vec![kern(t), fraction_rule(t), kern(k), boxed(b)], NATURAL)
}

/// `char_box` (§709).
fn char_box(eng: &Eng, f: usize, c: u32) -> BoxNode {
    let fonts = &eng.stomach.fonts;
    let mut b = BoxNode::new(BoxDir::H);
    b.width = fonts.width(f, c) + fonts.italic(f, c);
    b.height = fonts.height(f, c);
    b.depth = fonts.depth(f, c);
    b.list = vec![Node::Char { font: f, ch: c }];
    b
}

/// `stack_into_box` (§711).
fn stack_into_box(eng: &Eng, b: &mut BoxNode, f: usize, c: u32) {
    let p = char_box(eng, f, c);
    b.height = p.height;
    b.list.insert(0, boxed(p));
}

fn height_plus_depth(eng: &Eng, f: usize, c: u32) -> Scaled {
    eng.stomach.fonts.height(f, c) + eng.stomach.fonts.depth(f, c)
}

fn char_tag(eng: &Eng, f: usize, c: u32) -> Tag {
    eng.stomach.fonts.get(f).map_or(Tag::None, |t| t.tag(c))
}

/// `var_delimiter` (§706): a delimiter of height plus depth at least `v`, if the fonts allow.
pub(crate) fn var_delimiter(eng: &mut Eng, d: &Delimiter, s: u16, v: Scaled) -> BoxNode {
    let mut best: Option<(usize, u32)> = None;
    let mut w = 0;
    let mut found_ext = false;
    'search: for (fam, ch) in [(d.small_fam, d.small_char), (d.large_fam, d.large_char)] {
        if fam == 0 && ch == 0 {
            continue;
        }
        let mut z = u16::from(fam) + s + 16;
        loop {
            z -= 16;
            let g = fam_fnt(eng, z % 16, z - z % 16);
            if g != 0 {
                let mut y = ch;
                while eng.stomach.fonts.has_char(g, y) {
                    let tag = char_tag(eng, g, y);
                    if let Tag::Extensible(_) = tag {
                        best = Some((g, y));
                        found_ext = true;
                        break 'search;
                    }
                    let u = height_plus_depth(eng, g, y);
                    if u > w {
                        best = Some((g, y));
                        w = u;
                        if u >= v {
                            break 'search;
                        }
                    }
                    match tag {
                        Tag::List(next) => y = u32::from(next),
                        _ => break,
                    }
                }
            }
            if z < 16 {
                break;
            }
        }
    }
    let mut b = match best {
        Some((f, c)) => match char_tag(eng, f, c) {
            Tag::Extensible(r) if found_ext => extensible(eng, f, r, v),
            _ => char_box(eng, f, c),
        },
        None => {
            let mut b = BoxNode::new(BoxDir::H);
            b.width = eng.eqtb().dimen_par(NULL_DELIMITER_SPACE);
            b
        }
    };
    b.shift = half(b.height - b.depth) - axis_height(eng, s);
    b
}

/// Constructs an extensible character (§713).
fn extensible(eng: &Eng, f: usize, recipe: u8, v: Scaled) -> BoxNode {
    let mut b = BoxNode::new(BoxDir::V);
    let Some(r) = eng.stomach.fonts.get(f).and_then(|t| t.extensible(recipe)) else {
        return b;
    };
    let rep = u32::from(r.rep);
    let u = height_plus_depth(eng, f, rep);
    let mut w = 0;
    b.width = eng.stomach.fonts.width(f, rep) + eng.stomach.fonts.italic(f, rep);
    for c in [r.bot, r.mid, r.top] {
        if c != 0 {
            w += height_plus_depth(eng, f, u32::from(c));
        }
    }
    let mut n = 0;
    if u > 0 {
        while w < v {
            w += u;
            n += 1;
            if r.mid != 0 {
                w += u;
            }
        }
    }
    if r.bot != 0 {
        stack_into_box(eng, &mut b, f, u32::from(r.bot));
    }
    for _ in 0..n {
        stack_into_box(eng, &mut b, f, rep);
    }
    if r.mid != 0 {
        stack_into_box(eng, &mut b, f, u32::from(r.mid));
        for _ in 0..n {
            stack_into_box(eng, &mut b, f, rep);
        }
    }
    if r.top != 0 {
        stack_into_box(eng, &mut b, f, u32::from(r.top));
    }
    b.depth = w - b.height;
    b
}

/// `rebox` (§715).
fn rebox(eng: &mut Eng, mut b: BoxNode, w: Scaled) -> BoxNode {
    if b.width != w && !b.list.is_empty() {
        if b.dir == BoxDir::V {
            b = hpack(eng, vec![boxed(b)], NATURAL);
        }
        let mut list = std::mem::take(&mut b.list);
        if let [Node::Char { font, ch }] = list.as_slice() {
            let v = eng.stomach.fonts.width(*font, *ch);
            if v != b.width {
                list.push(kern(b.width - v));
            }
        }
        list.insert(0, Node::Glue(GlueNode::new(fil_glue(2))));
        list.push(Node::Glue(GlueNode::new(fil_glue(2))));
        hpack(eng, list, Spec::Exactly(w))
    } else {
        b.width = w;
        b
    }
}

fn mu_mult(n: i32, f: Scaled, x: Scaled) -> Scaled {
    nx_plus_y(n, x, xn_over_d(x, f, 0o200000).quotient).value
}

fn mu_split(m: Scaled) -> (i32, Scaled) {
    let d = x_over_n(m, 0o200000);
    let (mut n, mut f) = (d.quotient, d.remainder);
    if f < 0 {
        n -= 1;
        f += 0o200000;
    }
    (n, f)
}

/// `math_glue` (§716).
fn math_glue(g: &Glue, m: Scaled) -> Glue {
    let (n, f) = mu_split(m);
    Glue {
        width: mu_mult(n, f, g.width),
        stretch: if g.stretch_order == GlueOrder::Normal {
            mu_mult(n, f, g.stretch)
        } else {
            g.stretch
        },
        stretch_order: g.stretch_order,
        shrink: if g.shrink_order == GlueOrder::Normal {
            mu_mult(n, f, g.shrink)
        } else {
            g.shrink
        },
        shrink_order: g.shrink_order,
    }
}

/// `fetch` (§722): the font and character of a math character field, if the character exists.
fn fetch(eng: &mut Eng, cx: &Cx, a: &mut MathField) -> Res<Option<(usize, u32)>> {
    let (fam, c) = match *a {
        MathField::Char { fam, ch } | MathField::TextChar { fam, ch } => (fam, ch),
        _ => return Ok(None),
    };
    let f = fam_fnt(eng, u16::from(fam), cx.size);
    if f == 0 {
        eng.print_err("");
        eng.print_size(cx.size);
        eng.print_char(u32::from(' '));
        eng.print_int(i32::from(fam));
        eng.print_str(" is undefined (character ");
        eng.print(c);
        eng.print_char(u32::from(')'));
        eng.set_help(&[
            "Somewhere in the math formula just ended, you used the",
            "stated character from an undefined font family. For example,",
            "plain TeX doesn't allow \\it or \\sl in subscripts. Proceed,",
            "and I'll try to forget that I needed that character.",
        ]);
        eng.error()?;
        *a = MathField::Empty;
        return Ok(None);
    }
    if !eng.stomach.fonts.has_char(f, c) {
        char_warning(eng, f, c);
        *a = MathField::Empty;
        return Ok(None);
    }
    Ok(Some((f, c)))
}

/// `clean_box` (§720).
fn clean_box(eng: &mut Eng, field: MathField, style: u8) -> Res<BoxNode> {
    let q = match field {
        MathField::Char { .. } | MathField::TextChar { .. } => {
            let mut n = Noad::new(NoadKind::Ord);
            n.nucleus = field;
            mlist_to_hlist(eng, vec![Node::Noad(Box::new(n))], style, false)?
        }
        MathField::SubBox(b) => vec![Node::Box(b)],
        MathField::SubMlist(list) => mlist_to_hlist(eng, list, style, false)?,
        MathField::Empty => vec![boxed(BoxNode::new(BoxDir::H))],
    };
    let mut x = match <[Node; 1]>::try_from(q) {
        Ok([Node::Box(b)]) if b.shift == 0 => *b,
        Ok([n]) => hpack(eng, vec![n], NATURAL),
        Err(q) => hpack(eng, q, NATURAL),
    };
    if let [Node::Char { .. }, Node::Kern { .. }] = x.list.as_slice() {
        x.list.pop();
    }
    Ok(x)
}

/// What the first pass does with an item (§730).
enum Item {
    Style(u8),
    Choice,
    Pass,
    Rule(Scaled, Scaled),
    Glue,
    Kern,
    Fraction,
    Noad(NoadClass),
}

fn item_of(n: &Node) -> Option<Item> {
    Some(match n {
        Node::Style(s) => Item::Style(*s),
        Node::Choice(_) => Item::Choice,
        Node::Ins(_) | Node::Mark(_) | Node::Adjust(_) | Node::Whatsit(_) | Node::Penalty(_) | Node::Disc(_) => {
            Item::Pass
        }
        Node::Rule { height, depth, .. } => Item::Rule(*height, *depth),
        Node::Glue(_) => Item::Glue,
        Node::Kern { .. } => Item::Kern,
        Node::Fraction(_) => Item::Fraction,
        Node::Noad(_) => Item::Noad(noad_class(n)),
        _ => return None,
    })
}

/// `mlist_to_hlist` (§726): converts `mlist` in `style` into an hlist.
pub(crate) fn mlist_to_hlist(eng: &mut Eng, mut mlist: Vec<Node>, style: u8, penalties: bool) -> Res<Vec<Node>> {
    let mut cx = Cx::new(eng, style);
    let mut r: Option<usize> = None;
    let mut r_type = NoadClass::Op;
    let mut max_h = 0;
    let mut max_d = 0;
    let mut i = 0;
    while i < mlist.len() {
        let Some(item) = item_of(&mlist[i]) else {
            eng.confusion("mlist1")?;
            return Ok(Vec::new());
        };
        let check = match item {
            Item::Style(s) => {
                cx.set_style(eng, s);
                i += 1;
                continue;
            }
            Item::Choice => {
                let Node::Choice(lists) = std::mem::replace(&mut mlist[i], Node::Style(cx.style)) else {
                    unreachable!()
                };
                let [d, t, s, ss] = *lists;
                let chosen = match cx.style / 2 {
                    0 => d,
                    1 => t,
                    2 => s,
                    _ => ss,
                };
                mlist.splice(i + 1..i + 1, chosen);
                i += 1;
                continue;
            }
            Item::Pass => {
                i += 1;
                continue;
            }
            Item::Rule(h, d) => {
                max_h = max_h.max(h);
                max_d = max_d.max(d);
                i += 1;
                continue;
            }
            Item::Glue => {
                let remove_next = match &mut mlist[i] {
                    Node::Glue(g) if g.kind == GlueKind::Mu => {
                        g.spec = math_glue(&g.spec, cx.mu);
                        g.kind = GlueKind::Normal;
                        false
                    }
                    Node::Glue(g) => cx.size != TEXT_SIZE && g.kind == GlueKind::CondMath,
                    _ => false,
                };
                if remove_next && matches!(mlist.get(i + 1), Some(Node::Glue(_) | Node::Kern { .. })) {
                    mlist.remove(i + 1);
                }
                i += 1;
                continue;
            }
            Item::Kern => {
                if let Node::Kern { width, kind } = &mut mlist[i]
                    && *kind == KernKind::Mu
                {
                    let (n, f) = mu_split(cx.mu);
                    *width = mu_mult(n, f, *width);
                    *kind = KernKind::Explicit;
                }
                i += 1;
                continue;
            }
            Item::Fraction => {
                make_fraction(eng, &cx, &mut mlist[i])?;
                true
            }
            Item::Noad(mut kind) => {
                if kind == NoadClass::Bin
                    && matches!(
                        r_type,
                        NoadClass::Bin
                            | NoadClass::Op
                            | NoadClass::Rel
                            | NoadClass::Open
                            | NoadClass::Punct
                            | NoadClass::Left
                    )
                {
                    set_ord(&mut mlist[i]);
                    kind = NoadClass::Ord;
                }
                if matches!(
                    kind,
                    NoadClass::Rel | NoadClass::Close | NoadClass::Punct | NoadClass::Right
                ) && r_type == NoadClass::Bin
                    && let Some(r) = r
                {
                    set_ord(&mut mlist[r]);
                }
                match kind {
                    NoadClass::Left | NoadClass::Right => false,
                    NoadClass::Op => {
                        let delta = make_op(eng, &cx, &mut mlist[i])?;
                        if noad(&mlist[i]).limits != Limits::Limits {
                            convert_nucleus(eng, &cx, &mut mlist[i], delta)?;
                        }
                        true
                    }
                    NoadClass::Ord => {
                        make_ord(eng, &cx, &mut mlist, i)?;
                        convert_nucleus(eng, &cx, &mut mlist[i], 0)?;
                        true
                    }
                    NoadClass::Other(k) => {
                        let n = &mut mlist[i];
                        match k {
                            OtherNoad::Radical => make_radical(eng, &cx, n)?,
                            OtherNoad::Over => make_over(eng, &cx, n)?,
                            OtherNoad::Under => make_under(eng, &cx, n)?,
                            OtherNoad::Accent => make_math_accent(eng, &cx, n)?,
                            OtherNoad::VCenter => make_vcenter(eng, &cx, n)?,
                        }
                        convert_nucleus(eng, &cx, &mut mlist[i], 0)?;
                        true
                    }
                    _ => {
                        convert_nucleus(eng, &cx, &mut mlist[i], 0)?;
                        true
                    }
                }
            }
        };
        if check {
            let hl = new_hlist(&mlist[i]).cloned().unwrap_or_default();
            let z = hpack(eng, hl, NATURAL);
            max_h = max_h.max(z.height);
            max_d = max_d.max(z.depth);
        }
        r = Some(i);
        r_type = noad_class(&mlist[i]);
        i += 1;
    }
    if r_type == NoadClass::Bin
        && let Some(r) = r
    {
        set_ord(&mut mlist[r]);
    }
    second_pass(eng, mlist, style, penalties, max_h, max_d)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OtherNoad {
    Radical,
    Over,
    Under,
    Accent,
    VCenter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoadClass {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
    Fraction,
    Left,
    Right,
    Other(OtherNoad),
}

impl NoadClass {
    fn spacing_index(self) -> usize {
        match self {
            NoadClass::Op => 1,
            NoadClass::Bin => 2,
            NoadClass::Rel => 3,
            NoadClass::Open => 4,
            NoadClass::Close => 5,
            NoadClass::Punct => 6,
            NoadClass::Inner => 7,
            _ => 0,
        }
    }
}

fn noad_class(n: &Node) -> NoadClass {
    match n {
        Node::Fraction(_) => NoadClass::Fraction,
        Node::Noad(q) => match q.kind {
            NoadKind::Ord => NoadClass::Ord,
            NoadKind::Op => NoadClass::Op,
            NoadKind::Bin => NoadClass::Bin,
            NoadKind::Rel => NoadClass::Rel,
            NoadKind::Open => NoadClass::Open,
            NoadKind::Close => NoadClass::Close,
            NoadKind::Punct => NoadClass::Punct,
            NoadKind::Inner => NoadClass::Inner,
            NoadKind::Radical(_) => NoadClass::Other(OtherNoad::Radical),
            NoadKind::Over => NoadClass::Other(OtherNoad::Over),
            NoadKind::Under => NoadClass::Other(OtherNoad::Under),
            NoadKind::Accent { .. } => NoadClass::Other(OtherNoad::Accent),
            NoadKind::VCenter => NoadClass::Other(OtherNoad::VCenter),
            NoadKind::Left(_) => NoadClass::Left,
            NoadKind::Right { .. } => NoadClass::Right,
        },
        _ => NoadClass::Ord,
    }
}

fn set_ord(n: &mut Node) {
    if let Node::Noad(q) = n {
        q.kind = NoadKind::Ord;
    }
}

fn noad(n: &Node) -> &Noad {
    match n {
        Node::Noad(q) => q,
        _ => unreachable!("a noad was expected"),
    }
}

fn noad_mut(n: &mut Node) -> &mut Noad {
    match n {
        Node::Noad(q) => q,
        _ => unreachable!("a noad was expected"),
    }
}

fn new_hlist(n: &Node) -> Option<&Vec<Node>> {
    match n {
        Node::Noad(q) => q.new_hlist.as_ref(),
        Node::Fraction(f) => f.new_hlist.as_ref(),
        _ => None,
    }
}

/// `make_over` (§734).
fn make_over(eng: &mut Eng, cx: &Cx, n: &mut Node) -> Res {
    let nucleus = std::mem::take(&mut noad_mut(n).nucleus);
    let t = cx.rule_thickness(eng);
    let x = clean_box(eng, nucleus, cramped_style(cx.style))?;
    let b = overbar(eng, x, 3 * t, t);
    noad_mut(n).nucleus = MathField::SubBox(Box::new(b));
    Ok(())
}

/// `make_under` (§735).
fn make_under(eng: &mut Eng, cx: &Cx, n: &mut Node) -> Res {
    let nucleus = std::mem::take(&mut noad_mut(n).nucleus);
    let t = cx.rule_thickness(eng);
    let x = clean_box(eng, nucleus, cx.style)?;
    let xh = x.height;
    let mut y = vpack(eng, vec![boxed(x), kern(3 * t), fraction_rule(t)], NATURAL);
    let delta = y.height + y.depth + t;
    y.height = xh;
    y.depth = delta - y.height;
    noad_mut(n).nucleus = MathField::SubBox(Box::new(y));
    Ok(())
}

/// `make_vcenter` (§736).
fn make_vcenter(eng: &mut Eng, cx: &Cx, n: &mut Node) -> Res {
    let axis = axis_height(eng, cx.size);
    match &mut noad_mut(n).nucleus {
        MathField::SubBox(v) if v.dir == BoxDir::V => {
            let delta = v.height + v.depth;
            v.height = axis + half(delta);
            v.depth = delta - v.height;
            Ok(())
        }
        _ => eng.confusion("vcenter"),
    }
}

/// `make_radical` (§737).
fn make_radical(eng: &mut Eng, cx: &Cx, n: &mut Node) -> Res {
    let q = noad_mut(n);
    let nucleus = std::mem::take(&mut q.nucleus);
    let delim = match q.kind {
        NoadKind::Radical(d) => d,
        _ => Delimiter::default(),
    };
    let x = clean_box(eng, nucleus, cramped_style(cx.style))?;
    let t = cx.rule_thickness(eng);
    let mut clr = if cx.style < TEXT_STYLE {
        t + cx.sy(eng, 5).abs() / 4
    } else {
        t + t.abs() / 4
    };
    let mut y = var_delimiter(eng, &delim, cx.size, x.height + x.depth + clr + t);
    let delta = y.depth - (x.height + x.depth + clr);
    if delta > 0 {
        clr += half(delta);
    }
    y.shift = -(x.height + clr);
    let yh = y.height;
    let bar = overbar(eng, x, clr, yh);
    let b = hpack(eng, vec![boxed(y), boxed(bar)], NATURAL);
    noad_mut(n).nucleus = MathField::SubBox(Box::new(b));
    Ok(())
}

/// `make_math_accent` (§738).
fn make_math_accent(eng: &mut Eng, cx: &Cx, n: &mut Node) -> Res {
    let (afam, ach) = match noad(n).kind {
        NoadKind::Accent { fam, ch } => (fam, ch),
        _ => return Ok(()),
    };
    let mut accent = MathField::Char { fam: afam, ch: ach };
    let Some((f, mut c)) = fetch(eng, cx, &mut accent)? else {
        return Ok(());
    };
    let mut s = 0;
    if matches!(noad(n).nucleus, MathField::Char { .. }) {
        let mut nucleus = noad(n).nucleus.clone();
        let fetched = fetch(eng, cx, &mut nucleus)?;
        noad_mut(n).nucleus = nucleus;
        if let Some((nf, nc)) = fetched {
            s = skew(eng, nf, nc);
        }
    }
    let nucleus_is_char = matches!(noad(n).nucleus, MathField::Char { .. });
    let nucleus = std::mem::take(&mut noad_mut(n).nucleus);
    let mut x = clean_box(eng, nucleus.clone(), cramped_style(cx.style))?;
    let w = x.width;
    let mut h = x.height;
    while let Tag::List(y) = char_tag(eng, f, c) {
        let y = u32::from(y);
        if !eng.stomach.fonts.has_char(f, y) || eng.stomach.fonts.width(f, y) > w {
            break;
        }
        c = y;
    }
    let xh = param(eng, f, 4);
    let mut delta = if h < xh { h } else { xh };
    let q = noad_mut(n);
    if (q.supscr != MathField::Empty || q.subscr != MathField::Empty) && nucleus_is_char {
        let mut inner = Noad::new(NoadKind::Ord);
        inner.nucleus = nucleus;
        inner.supscr = std::mem::take(&mut q.supscr);
        inner.subscr = std::mem::take(&mut q.subscr);
        let field = MathField::SubMlist(vec![Node::Noad(Box::new(inner))]);
        x = clean_box(eng, field, cx.style)?;
        delta = delta + x.height - h;
        h = x.height;
    }
    let mut y = char_box(eng, f, c);
    y.shift = s + half(w - y.width);
    y.width = 0;
    let xw = x.width;
    let mut y = vpack(eng, vec![boxed(y), kern(-delta), boxed(x)], NATURAL);
    y.width = xw;
    if y.height < h {
        y.list.insert(0, kern(h - y.height));
        y.height = h;
    }
    noad_mut(n).nucleus = MathField::SubBox(Box::new(y));
    Ok(())
}

/// The kern between `c` and the font's `\skewchar` (§741).
fn skew(eng: &Eng, f: usize, c: u32) -> Scaled {
    let skew_char = eng.fonts().get(f).map_or(-1, |i| i.skew_char);
    let Some(tfm) = eng.stomach.fonts.get(f) else { return 0 };
    let Some(mut a) = tfm.program_of(c) else { return 0 };
    while let Some(step) = tfm.step(a) {
        if i32::from(step.next_char) == skew_char {
            if step.is_kern() && step.skip <= 128 {
                return tfm.kern(&step);
            }
            return 0;
        }
        if step.skip >= 128 {
            return 0;
        }
        a += usize::from(step.skip) + 1;
    }
    0
}

/// `make_fraction` (§743).
fn make_fraction(eng: &mut Eng, cx: &Cx, n: &mut Node) -> Res {
    let Node::Fraction(q) = n else { return Ok(()) };
    let thickness = q.thickness.unwrap_or_else(|| cx.rule_thickness(eng));
    q.thickness = Some(thickness);
    let num = std::mem::take(&mut q.numerator);
    let den = std::mem::take(&mut q.denominator);
    let (left, right) = (q.left, q.right);
    let x = clean_box(eng, num, num_style(cx.style))?;
    let z = clean_box(eng, den, denom_style(cx.style))?;
    let (x, z) = if x.width < z.width {
        let w = z.width;
        (rebox(eng, x, w), z)
    } else {
        let w = x.width;
        (x, rebox(eng, z, w))
    };
    let (mut shift_up, mut shift_down) = if cx.style < TEXT_STYLE {
        (cx.sy(eng, 8), cx.sy(eng, 11))
    } else {
        (
            if thickness != 0 { cx.sy(eng, 9) } else { cx.sy(eng, 10) },
            cx.sy(eng, 12),
        )
    };
    let axis = axis_height(eng, cx.size);
    let delta;
    if thickness == 0 {
        let t = cx.rule_thickness(eng);
        let clr = if cx.style < TEXT_STYLE { 7 * t } else { 3 * t };
        delta = half(clr - ((shift_up - x.depth) - (z.height - shift_down)));
        if delta > 0 {
            shift_up += delta;
            shift_down += delta;
        }
    } else {
        let clr = if cx.style < TEXT_STYLE {
            3 * thickness
        } else {
            thickness
        };
        delta = half(thickness);
        let delta1 = clr - ((shift_up - x.depth) - (axis + delta));
        let delta2 = clr - ((axis - delta) - (z.height - shift_down));
        if delta1 > 0 {
            shift_up += delta1;
        }
        if delta2 > 0 {
            shift_down += delta2;
        }
    }
    let mut v = BoxNode::new(BoxDir::V);
    v.height = shift_up + x.height;
    v.depth = z.depth + shift_down;
    v.width = x.width;
    v.list = if thickness == 0 {
        let k = (shift_up - x.depth) - (z.height - shift_down);
        vec![boxed(x), kern(k), boxed(z)]
    } else {
        let k1 = (shift_up - x.depth) - (axis + delta);
        let k2 = (axis - delta) - (z.height - shift_down);
        vec![boxed(x), kern(k1), fraction_rule(thickness), kern(k2), boxed(z)]
    };
    let d = if cx.style < TEXT_STYLE {
        cx.sy(eng, 20)
    } else {
        cx.sy(eng, 21)
    };
    let l = var_delimiter(eng, &left, cx.size, d);
    let r = var_delimiter(eng, &right, cx.size, d);
    let b = hpack(eng, vec![boxed(l), boxed(v), boxed(r)], NATURAL);
    if let Node::Fraction(q) = n {
        q.new_hlist = Some(vec![boxed(b)]);
    }
    Ok(())
}

/// `make_op` (§749): returns the italic correction that offsets the limits.
fn make_op(eng: &mut Eng, cx: &Cx, n: &mut Node) -> Res<Scaled> {
    let q = noad_mut(n);
    if q.limits == Limits::Normal && cx.style < TEXT_STYLE {
        q.limits = Limits::Limits;
    }
    let mut delta = 0;
    if matches!(q.nucleus, MathField::Char { .. }) {
        let mut nucleus = std::mem::take(&mut q.nucleus);
        let fetched = fetch(eng, cx, &mut nucleus)?;
        if let (Some((f, mut c)), MathField::Char { fam, .. }) = (fetched, nucleus.clone()) {
            if cx.style < TEXT_STYLE
                && let Tag::List(next) = char_tag(eng, f, c)
                && eng.stomach.fonts.has_char(f, u32::from(next))
            {
                c = u32::from(next);
                nucleus = MathField::Char { fam, ch: c };
            }
            delta = eng.stomach.fonts.italic(f, c);
        }
        let mut x = clean_box(eng, nucleus, cx.style)?;
        let q = noad_mut(n);
        if q.subscr != MathField::Empty && q.limits != Limits::Limits {
            x.width -= delta;
        }
        x.shift = half(x.height - x.depth) - axis_height(eng, cx.size);
        noad_mut(n).nucleus = MathField::SubBox(Box::new(x));
    }
    if noad(n).limits == Limits::Limits {
        let q = noad_mut(n);
        let sup = std::mem::take(&mut q.supscr);
        let nucleus = std::mem::take(&mut q.nucleus);
        let sub = std::mem::take(&mut q.subscr);
        let (has_sup, has_sub) = (sup != MathField::Empty, sub != MathField::Empty);
        let x = clean_box(eng, sup, sup_style(cx.style))?;
        let y = clean_box(eng, nucleus, cx.style)?;
        let z = clean_box(eng, sub, sub_style(cx.style))?;
        let mut v = BoxNode::new(BoxDir::V);
        v.width = y.width.max(x.width).max(z.width);
        let w = v.width;
        let mut x = rebox(eng, x, w);
        let y = rebox(eng, y, w);
        let mut z = rebox(eng, z, w);
        x.shift = half(delta);
        z.shift = -x.shift;
        v.height = y.height;
        v.depth = y.depth;
        let mut list = Vec::new();
        if has_sup {
            let shift_up = (cx.ex(eng, 11) - x.depth).max(cx.ex(eng, 9));
            v.height += cx.ex(eng, 13) + x.height + x.depth + shift_up;
            list.push(kern(cx.ex(eng, 13)));
            list.push(boxed(x));
            list.push(kern(shift_up));
        }
        list.push(boxed(y));
        if has_sub {
            let shift_down = (cx.ex(eng, 12) - z.height).max(cx.ex(eng, 10));
            v.depth += cx.ex(eng, 13) + z.height + z.depth + shift_down;
            list.push(kern(shift_down));
            list.push(boxed(z));
            list.push(kern(cx.ex(eng, 13)));
        }
        v.list = list;
        noad_mut(n).new_hlist = Some(vec![boxed(v)]);
    }
    Ok(delta)
}

/// `make_ord` (§752): ligatures and kerns between adjacent characters of the same family.
fn make_ord(eng: &mut Eng, cx: &Cx, list: &mut Vec<Node>, i: usize) -> Res {
    loop {
        let q = noad(&list[i]);
        let (MathField::Empty, MathField::Empty, MathField::Char { fam, ch: _ }) = (&q.subscr, &q.supscr, &q.nucleus)
        else {
            return Ok(());
        };
        let fam = *fam;
        let cur_c = match list.get(i + 1) {
            Some(Node::Noad(p))
                if matches!(
                    p.kind,
                    NoadKind::Ord
                        | NoadKind::Op
                        | NoadKind::Bin
                        | NoadKind::Rel
                        | NoadKind::Open
                        | NoadKind::Close
                        | NoadKind::Punct
                ) =>
            {
                match p.nucleus {
                    MathField::Char { fam: pf, ch } if pf == fam => ch,
                    _ => return Ok(()),
                }
            }
            _ => return Ok(()),
        };
        let q = noad_mut(&mut list[i]);
        if let MathField::Char { fam, ch } = q.nucleus {
            q.nucleus = MathField::TextChar { fam, ch };
        }
        let mut nucleus = q.nucleus.clone();
        let fetched = fetch(eng, cx, &mut nucleus)?;
        noad_mut(&mut list[i]).nucleus = nucleus;
        let Some((f, c)) = fetched else { return Ok(()) };
        let Some(tfm) = eng.stomach.fonts.get(f) else {
            return Ok(());
        };
        let Some(mut a) = tfm.program_of(c) else { return Ok(()) };
        loop {
            let Some(step) = eng.stomach.fonts.get(f).and_then(|t| t.step(a)) else {
                return Ok(());
            };
            if u32::from(step.next_char) == cur_c && step.skip <= 128 {
                if step.is_kern() {
                    let k = eng.stomach.fonts.get(f).map_or(0, |t| t.kern(&step));
                    list.insert(i + 1, kern(k));
                    return Ok(());
                }
                let rem = u32::from(step.remainder);
                match step.op {
                    1 | 5 => set_char(&mut noad_mut(&mut list[i]).nucleus, rem),
                    2 | 6 => set_char(&mut noad_mut(&mut list[i + 1]).nucleus, rem),
                    3 | 7 | 11 => {
                        let mut r = Noad::new(NoadKind::Ord);
                        r.nucleus = if step.op < 11 {
                            MathField::Char { fam, ch: rem }
                        } else {
                            MathField::TextChar { fam, ch: rem }
                        };
                        list.insert(i + 1, Node::Noad(Box::new(r)));
                    }
                    _ => {
                        let Node::Noad(p) = list.remove(i + 1) else {
                            unreachable!()
                        };
                        let q = noad_mut(&mut list[i]);
                        set_char(&mut q.nucleus, rem);
                        q.subscr = p.subscr;
                        q.supscr = p.supscr;
                    }
                }
                if step.op > 3 {
                    return Ok(());
                }
                let q = noad_mut(&mut list[i]);
                if let MathField::TextChar { fam, ch } = q.nucleus {
                    q.nucleus = MathField::Char { fam, ch };
                }
                break;
            }
            if step.skip >= 128 {
                return Ok(());
            }
            a += usize::from(step.skip) + 1;
        }
    }
}

fn set_char(field: &mut MathField, c: u32) {
    if let MathField::Char { ch, .. } | MathField::TextChar { ch, .. } = field {
        *ch = c;
    }
}

/// Converts the nucleus to an hlist and attaches the scripts (§754).
fn convert_nucleus(eng: &mut Eng, cx: &Cx, n: &mut Node, mut delta: Scaled) -> Res {
    let q = noad_mut(n);
    let has_sub = q.subscr != MathField::Empty;
    let mut nucleus = std::mem::take(&mut q.nucleus);
    let p = match nucleus {
        MathField::Char { .. } | MathField::TextChar { .. } => {
            let text = matches!(nucleus, MathField::TextChar { .. });
            match fetch(eng, cx, &mut nucleus)? {
                Some((f, c)) => {
                    delta = eng.stomach.fonts.italic(f, c);
                    let mut p: Vec<Node> = new_character(eng, f, c).into_iter().collect();
                    if text && param(eng, f, 1) != 0 {
                        delta = 0;
                    }
                    if !has_sub && delta != 0 {
                        p.push(kern(delta));
                        delta = 0;
                    }
                    p
                }
                None => Vec::new(),
            }
        }
        MathField::Empty => Vec::new(),
        MathField::SubBox(b) => vec![Node::Box(b)],
        MathField::SubMlist(list) => {
            let hl = mlist_to_hlist(eng, list, cx.style, false)?;
            vec![boxed(hpack(eng, hl, NATURAL))]
        }
    };
    let q = noad_mut(n);
    q.new_hlist = Some(p);
    if q.subscr == MathField::Empty && q.supscr == MathField::Empty {
        return Ok(());
    }
    make_scripts(eng, cx, n, delta)
}

/// `make_scripts` (§756).
fn make_scripts(eng: &mut Eng, cx: &Cx, n: &mut Node, delta: Scaled) -> Res {
    let q = noad_mut(n);
    let p = q.new_hlist.clone().unwrap_or_default();
    let sup = std::mem::take(&mut q.supscr);
    let sub = std::mem::take(&mut q.subscr);
    let (mut shift_up, mut shift_down) = if matches!(p.first(), Some(Node::Char { .. })) {
        (0, 0)
    } else {
        let z = hpack(eng, p, NATURAL);
        let t = if cx.style < SCRIPT_STYLE {
            SCRIPT_SIZE
        } else {
            SCRIPT_SCRIPT_SIZE
        };
        (z.height - mathsy(eng, 18, t), z.depth + mathsy(eng, 19, t))
    };
    let script_space = eng.eqtb().dimen_par(SCRIPT_SPACE);
    let x_height = cx.sy(eng, 5);
    let x = if sup == MathField::Empty {
        let mut x = clean_box(eng, sub, sub_style(cx.style))?;
        x.width += script_space;
        shift_down = shift_down.max(cx.sy(eng, 16));
        let clr = x.height - (x_height * 4).abs() / 5;
        shift_down = shift_down.max(clr);
        x.shift = shift_down;
        x
    } else {
        let mut x = clean_box(eng, sup, sup_style(cx.style))?;
        x.width += script_space;
        let clr = if cx.style % 2 == 1 {
            cx.sy(eng, 15)
        } else if cx.style < TEXT_STYLE {
            cx.sy(eng, 13)
        } else {
            cx.sy(eng, 14)
        };
        shift_up = shift_up.max(clr);
        shift_up = shift_up.max(x.depth + x_height.abs() / 4);
        if sub == MathField::Empty {
            x.shift = -shift_up;
            x
        } else {
            let mut y = clean_box(eng, sub, sub_style(cx.style))?;
            y.width += script_space;
            shift_down = shift_down.max(cx.sy(eng, 17));
            let clr = 4 * cx.rule_thickness(eng) - ((shift_up - x.depth) - (y.height - shift_down));
            if clr > 0 {
                shift_down += clr;
                let clr = (x_height * 4).abs() / 5 - (shift_up - x.depth);
                if clr > 0 {
                    shift_up += clr;
                    shift_down -= clr;
                }
            }
            x.shift = delta;
            let k = (shift_up - x.depth) - (y.height - shift_down);
            let mut v = vpack(eng, vec![boxed(x), kern(k), boxed(y)], NATURAL);
            v.shift = shift_down;
            v
        }
    };
    noad_mut(n).new_hlist.get_or_insert_with(Vec::new).push(boxed(x));
    Ok(())
}

/// `make_left_right` (§762, with e-TeX's reset of the style).
fn make_left_right(eng: &mut Eng, cx: &mut Cx, n: &mut Node, style: u8, max_d: Scaled, max_h: Scaled) -> NoadClass {
    cx.set_style(eng, style);
    let mut delta2 = max_d + axis_height(eng, cx.size);
    let mut delta1 = max_h + max_d - delta2;
    if delta2 > delta1 {
        delta1 = delta2;
    }
    let mut delta = (delta1 / 500) * eng.int_par(DELIMITER_FACTOR);
    delta2 = delta1 + delta1 - eng.eqtb().dimen_par(DELIMITER_SHORTFALL);
    if delta < delta2 {
        delta = delta2;
    }
    let q = noad_mut(n);
    let (d, class) = match q.kind {
        NoadKind::Left(d) => (d, NoadClass::Open),
        NoadKind::Right { delim, .. } => (delim, NoadClass::Close),
        _ => (Delimiter::default(), NoadClass::Ord),
    };
    let b = var_delimiter(eng, &d, cx.size, delta);
    noad_mut(n).new_hlist = Some(vec![boxed(b)]);
    class
}

/// The second pass of `mlist_to_hlist` (§760): spacing and penalties.
fn second_pass(
    eng: &mut Eng,
    mlist: Vec<Node>,
    style: u8,
    penalties: bool,
    max_h: Scaled,
    max_d: Scaled,
) -> Res<Vec<Node>> {
    let mut out = Vec::new();
    let mut cx = Cx::new(eng, style);
    let mut r_type: Option<NoadClass> = None;
    let mut items = mlist.into_iter().peekable();
    while let Some(mut q) = items.next() {
        let mut t = NoadClass::Ord;
        let mut pen = INF_PENALTY;
        match noad_class_of(&q) {
            Some(NoadClass::Op | NoadClass::Open | NoadClass::Close | NoadClass::Punct | NoadClass::Inner) => {
                t = noad_class(&q);
            }
            Some(NoadClass::Bin) => {
                t = NoadClass::Bin;
                pen = eng.int_par(BIN_OP_PENALTY);
            }
            Some(NoadClass::Rel) => {
                t = NoadClass::Rel;
                pen = eng.int_par(REL_PENALTY);
            }
            Some(NoadClass::Left | NoadClass::Right) => {
                t = make_left_right(eng, &mut cx, &mut q, style, max_d, max_h);
            }
            Some(_) => {}
            None => {
                if let Node::Style(s) = q {
                    cx.set_style(eng, s);
                } else {
                    out.push(q);
                }
                continue;
            }
        }
        if let Some(r) = r_type {
            let x = match MATH_SPACING[r.spacing_index() * 8 + t.spacing_index()] {
                b'0' => 0,
                b'1' if cx.style < SCRIPT_STYLE => THIN_MU_SKIP,
                b'1' => 0,
                b'2' => THIN_MU_SKIP,
                b'3' if cx.style < SCRIPT_STYLE => MED_MU_SKIP,
                b'4' if cx.style < SCRIPT_STYLE => THICK_MU_SKIP,
                b'3' | b'4' => 0,
                _ => {
                    eng.confusion("mlist4")?;
                    0
                }
            };
            if x != 0 {
                let spec = math_glue(eng.eqtb().glue_par(usize::from(x)).get(), cx.mu);
                out.push(Node::Glue(GlueNode {
                    spec,
                    kind: GlueKind::Param(x),
                    leader: None,
                    zero_ptr: false,
                }));
            }
        }
        if let Some(hl) = take_new_hlist(q) {
            out.extend(hl);
        }
        if penalties
            && pen < INF_PENALTY
            && let Some(next) = items.peek()
            && !matches!(next, Node::Penalty(_))
            && noad_class_of(next) != Some(NoadClass::Rel)
        {
            out.push(Node::Penalty(pen));
        }
        r_type = Some(t);
    }
    Ok(out)
}

fn noad_class_of(n: &Node) -> Option<NoadClass> {
    match n {
        Node::Noad(_) | Node::Fraction(_) => Some(noad_class(n)),
        _ => None,
    }
}

fn take_new_hlist(n: Node) -> Option<Vec<Node>> {
    match n {
        Node::Noad(q) => q.new_hlist,
        Node::Fraction(f) => f.new_hlist,
        _ => None,
    }
}

/// Whether the math fonts have enough parameters (§1195); prints the error and returns `false` otherwise.
pub(crate) fn check_fonts(eng: &mut Eng) -> Res<bool> {
    let short = |eng: &Eng, fam: u16, need: usize| {
        [TEXT_SIZE, SCRIPT_SIZE, SCRIPT_SCRIPT_SIZE].iter().any(|&s| {
            let f = fam_fnt(eng, fam, s);
            eng.fonts().get(f).map_or(0, |i| i.params.len()) < need
        })
    };
    if short(eng, 2, TOTAL_MATHSY_PARAMS) {
        eng.print_err("Math formula deleted: Insufficient symbol fonts");
        eng.set_help(&[
            "Sorry, but I can't typeset math unless \\textfont 2",
            "and \\scriptfont 2 and \\scriptscriptfont 2 have all",
            "the \\fontdimen values needed in math symbol fonts.",
        ]);
    } else if short(eng, 3, TOTAL_MATHEX_PARAMS) {
        eng.print_err("Math formula deleted: Insufficient extension fonts");
        eng.set_help(&[
            "Sorry, but I can't typeset math unless \\textfont 3",
            "and \\scriptfont 3 and \\scriptscriptfont 3 have all",
            "the \\fontdimen values needed in math extension fonts.",
        ]);
    } else {
        return Ok(true);
    }
    eng.error()?;
    Ok(false)
}

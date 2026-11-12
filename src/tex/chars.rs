//! The main loop for characters, with ligatures and kerns, and interword spaces (§1034–§1044).

use std::collections::VecDeque;

use rtex_core::cmd;
use rtex_core::{Glue, Res, xn_over_d};

use super::fonts::{char_warning, param};
use super::node::{DiscNode, GlueNode, KernKind, LigatureNode, Node, Whatsit};
use super::pack::param_glue;
use super::{Eng, abs_mode, tail_append};

const NON_CHAR: u32 = 256;
const LANGUAGE: usize = 50;
const LEFT_HYPHEN_MIN: usize = 51;
const RIGHT_HYPHEN_MIN: usize = 52;
const SPACE_SKIP: usize = 12;
const XSPACE_SKIP: usize = 13;

#[derive(Clone, Copy, Debug)]
enum LigItem {
    Char(u32),
    Pseudo { ch: u32, ptr: Option<u32> },
}

impl LigItem {
    fn ch(&self) -> u32 {
        match self {
            LigItem::Char(c) | LigItem::Pseudo { ch: c, .. } => *c,
        }
    }
}

/// `adjust_space_factor` (§1034).
pub(crate) fn adjust_space_factor(eng: &mut Eng, c: u32) {
    let s = eng.eqtb().sf_code(c) as i32;
    if s == 1000 {
        eng.aux.space_factor = 1000;
    } else if s < 1000 {
        if s > 0 {
            eng.aux.space_factor = s;
        }
    } else if eng.aux.space_factor < 1000 {
        eng.aux.space_factor = 1000;
    } else {
        eng.aux.space_factor = s;
    }
}

/// `fix_language` (§1376).
pub(crate) fn fix_language(eng: &mut Eng) {
    let l = eng.int_par(LANGUAGE);
    let l = if (1..=255).contains(&l) { l } else { 0 };
    if l != eng.stomach.clang {
        let lhm = eng.int_par(LEFT_HYPHEN_MIN).clamp(1, 63);
        let rhm = eng.int_par(RIGHT_HYPHEN_MIN).clamp(1, 63);
        tail_append(eng, Node::Whatsit(Whatsit::Language { lang: l, lhm, rhm }));
        eng.stomach.clang = l;
    }
}

enum Label {
    Wrapup,
    Move,
    Move1,
    Move2,
    Lookahead,
    LigLoop,
    LigLoop1,
    LigLoop2,
    MoveLig,
}

struct MainLoop {
    f: usize,
    bchar: u32,
    false_bchar: u32,
    cur_l: u32,
    cur_r: u32,
    cur_q: usize,
    lig_stack: VecDeque<LigItem>,
    k: usize,
    ligature_present: bool,
    lft_hit: bool,
    rt_hit: bool,
    ins_disc: bool,
}

/// The main loop for character `c` in horizontal mode (§1034); returns when a non-character
/// token is pending, which has been put back to be read again.
pub(crate) fn main_loop(eng: &mut Eng, c: u32) -> Res {
    adjust_space_factor(eng, c);
    let f = eng.eqtb().cur_font() as usize;
    let (bchar, false_bchar, label) = match eng.stomach.fonts.get(f) {
        Some(t) => (
            t.boundary_char().map_or(NON_CHAR, u32::from),
            t.false_boundary_char(),
            t.bchar_label(),
        ),
        None => (NON_CHAR, NON_CHAR, None),
    };
    if eng.mode > 0 && eng.int_par(LANGUAGE) != eng.stomach.clang {
        fix_language(eng);
    }
    let mut m = MainLoop {
        f,
        bchar,
        false_bchar,
        cur_l: c,
        cur_r: NON_CHAR,
        cur_q: eng.stomach.cur.len(),
        lig_stack: VecDeque::from([LigItem::Char(c)]),
        k: 0,
        ligature_present: false,
        lft_hit: false,
        rt_hit: false,
        ins_disc: false,
    };
    let mut cur_chr = c;
    let start_label = if eng.stomach.cancel_boundary {
        eng.stomach.cancel_boundary = false;
        None
    } else {
        label
    };
    let mut next = match start_label {
        None => Label::Move2,
        Some(k) => {
            m.cur_r = m.cur_l;
            m.cur_l = NON_CHAR;
            m.k = k;
            Label::LigLoop1
        }
    };
    loop {
        next = match next {
            Label::Wrapup => {
                m.wrapup(eng, m.rt_hit);
                Label::Move
            }
            Label::Move => {
                let Some(front) = m.lig_stack.front().copied() else {
                    eng.reswitch();
                    return Ok(());
                };
                m.cur_q = eng.stomach.cur.len();
                m.cur_l = front.ch();
                Label::Move1
            }
            Label::Move1 => match m.lig_stack.front() {
                Some(LigItem::Char(_)) => Label::Move2,
                _ => Label::MoveLig,
            },
            Label::Move2 => {
                let in_range = eng.stomach.fonts.get(m.f).is_some_and(|t| {
                    let (bc, ec) = t.char_range();
                    (u32::from(bc)..=u32::from(ec)).contains(&cur_chr)
                });
                if !in_range || !eng.stomach.fonts.has_char(m.f, m.cur_l) {
                    char_warning(eng, m.f, cur_chr);
                    return Ok(());
                }
                let ch = match m.lig_stack.pop_front() {
                    Some(LigItem::Char(c)) => c,
                    _ => m.cur_l,
                };
                tail_append(eng, Node::Char { font: m.f, ch });
                Label::Lookahead
            }
            Label::Lookahead => {
                eng.get_next()?;
                let mut c = None;
                if matches!(eng.cur_cmd(), cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN) {
                    c = Some(eng.cur_chr());
                } else {
                    eng.x_token()?;
                    match eng.cur_cmd() {
                        cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN => c = Some(eng.cur_chr()),
                        cmd::CHAR_NUM => c = Some(eng.scan_char_num()? as u32),
                        cmd::NO_BOUNDARY => m.bchar = NON_CHAR,
                        _ => {}
                    }
                }
                match c {
                    Some(ch) => {
                        cur_chr = ch;
                        adjust_space_factor(eng, ch);
                        m.lig_stack = VecDeque::from([LigItem::Char(ch)]);
                        m.cur_r = if ch == m.false_bchar { NON_CHAR } else { ch };
                    }
                    None => {
                        m.cur_r = m.bchar;
                        m.lig_stack.clear();
                    }
                }
                Label::LigLoop
            }
            Label::LigLoop => {
                let program = eng.stomach.fonts.get(m.f).and_then(|t| t.program_of(m.cur_l));
                match program {
                    Some(k) if m.cur_r != NON_CHAR => {
                        m.k = k;
                        Label::LigLoop1
                    }
                    _ => Label::Wrapup,
                }
            }
            Label::LigLoop1 => Label::LigLoop2,
            Label::LigLoop2 => {
                let Some(step) = eng.stomach.fonts.get(m.f).and_then(|t| t.step(m.k)) else {
                    next = Label::Wrapup;
                    continue;
                };
                if u32::from(step.next_char) == m.cur_r && step.skip <= 128 {
                    m.do_lig_kern(eng, step)?
                } else if step.skip == 0 {
                    m.k += 1;
                    Label::LigLoop1
                } else if step.skip >= 128 {
                    Label::Wrapup
                } else {
                    m.k += usize::from(step.skip) + 1;
                    Label::LigLoop1
                }
            }
            Label::MoveLig => {
                let Some(LigItem::Pseudo { ptr, .. }) = m.lig_stack.pop_front() else {
                    next = Label::Move2;
                    continue;
                };
                if let Some(p) = ptr {
                    tail_append(eng, Node::Char { font: m.f, ch: p });
                }
                m.ligature_present = true;
                match m.lig_stack.front() {
                    None => {
                        if ptr.is_some() {
                            Label::Lookahead
                        } else {
                            m.cur_r = m.bchar;
                            Label::LigLoop
                        }
                    }
                    Some(item) => {
                        m.cur_r = item.ch();
                        Label::LigLoop
                    }
                }
            }
        };
    }
}

impl MainLoop {
    fn pack_lig(&mut self, eng: &mut Eng, rt: bool) {
        let original: Vec<u32> = eng
            .stomach
            .cur
            .drain(self.cur_q..)
            .filter_map(|n| match n {
                Node::Char { ch, .. } => Some(ch),
                _ => None,
            })
            .collect();
        let mut lig = LigatureNode {
            font: self.f,
            ch: self.cur_l,
            original,
            left_hit: false,
            right_hit: false,
        };
        if self.lft_hit {
            lig.left_hit = true;
            self.lft_hit = false;
        }
        if rt && self.lig_stack.is_empty() {
            lig.right_hit = true;
            self.rt_hit = false;
        }
        eng.stomach.cur.push(Node::Ligature(lig));
        self.ligature_present = false;
    }

    fn wrapup(&mut self, eng: &mut Eng, rt: bool) {
        if self.cur_l < NON_CHAR {
            if eng.stomach.cur.len() > self.cur_q {
                let hyphen = eng.fonts().get(self.f).map_or(-1, |i| i.hyphen_char);
                if let Some(Node::Char { ch, .. }) = eng.stomach.cur.last()
                    && i64::from(*ch) == i64::from(hyphen)
                {
                    self.ins_disc = true;
                }
            }
            if self.ligature_present {
                self.pack_lig(eng, rt);
            }
            if self.ins_disc {
                self.ins_disc = false;
                if eng.mode > 0 {
                    tail_append(
                        eng,
                        Node::Disc(Box::new(DiscNode {
                            pre: Vec::new(),
                            post: Vec::new(),
                            replace_count: 0,
                        })),
                    );
                }
            }
        }
    }

    fn do_lig_kern(&mut self, eng: &mut Eng, step: crate::fonts::tfm::LigKernStep) -> Res<Label> {
        if step.is_kern() {
            self.wrapup(eng, self.rt_hit);
            let kern = eng.stomach.fonts.get(self.f).map_or(0, |t| t.kern(&step));
            tail_append(eng, Node::kern(kern, KernKind::Normal));
            return Ok(Label::Move);
        }
        if self.cur_l == NON_CHAR {
            self.lft_hit = true;
        } else if self.lig_stack.is_empty() {
            self.rt_hit = true;
        }
        let rem = u32::from(step.remainder);
        match step.op {
            1 | 5 => {
                self.cur_l = rem;
                self.ligature_present = true;
            }
            2 | 6 => {
                self.cur_r = rem;
                match self.lig_stack.front().copied() {
                    None => {
                        self.lig_stack.push_front(LigItem::Pseudo { ch: rem, ptr: None });
                        self.bchar = NON_CHAR;
                    }
                    Some(LigItem::Char(c)) => {
                        self.lig_stack[0] = LigItem::Pseudo { ch: rem, ptr: Some(c) };
                    }
                    Some(LigItem::Pseudo { ptr, .. }) => {
                        self.lig_stack[0] = LigItem::Pseudo { ch: rem, ptr };
                    }
                }
            }
            3 => {
                self.cur_r = rem;
                self.lig_stack.push_front(LigItem::Pseudo { ch: rem, ptr: None });
            }
            7 | 11 => {
                self.wrapup(eng, false);
                self.cur_q = eng.stomach.cur.len();
                self.cur_l = rem;
                self.ligature_present = true;
            }
            _ => {
                self.cur_l = rem;
                self.ligature_present = true;
                if self.lig_stack.is_empty() {
                    return Ok(Label::Wrapup);
                }
                return Ok(Label::Move1);
            }
        }
        if step.op > 4 && step.op != 7 {
            return Ok(Label::Wrapup);
        }
        if self.cur_l < NON_CHAR {
            return Ok(Label::LigLoop);
        }
        match eng.stomach.fonts.get(self.f).and_then(|t| t.bchar_label()) {
            Some(k) => {
                self.k = k;
                Ok(Label::LigLoop1)
            }
            None => Ok(Label::Wrapup),
        }
    }
}

/// The interword glue of the current font (`font_glue`, §1042).
fn font_glue(eng: &Eng) -> Glue {
    let f = eng.eqtb().cur_font() as usize;
    Glue {
        width: param(eng, f, 1),
        stretch: param(eng, f, 2),
        shrink: param(eng, f, 3),
        ..Glue::default()
    }
}

fn is_zero(g: &Glue) -> bool {
    g.width == 0 && g.stretch == 0 && g.shrink == 0
}

/// `append_normal_space` (§1041).
pub(crate) fn append_normal_space(eng: &mut Eng) {
    let space_skip = *eng.eqtb().glue_par(SPACE_SKIP).get();
    let node = if is_zero(&space_skip) {
        Node::Glue(GlueNode::new(font_glue(eng)))
    } else {
        param_glue(eng, SPACE_SKIP)
    };
    tail_append(eng, node);
}

/// `app_space` (§1043): a space when the space factor is not 1000.
pub(crate) fn app_space(eng: &mut Eng) {
    let sf = eng.aux.space_factor;
    let xspace = *eng.eqtb().glue_par(XSPACE_SKIP).get();
    let node = if sf >= 2000 && !is_zero(&xspace) {
        param_glue(eng, XSPACE_SKIP)
    } else {
        let space_skip = *eng.eqtb().glue_par(SPACE_SKIP).get();
        let mut g = if is_zero(&space_skip) {
            font_glue(eng)
        } else {
            space_skip
        };
        if sf >= 2000 {
            let f = eng.eqtb().cur_font() as usize;
            g.width += param(eng, f, 6);
        }
        g.stretch = xn_over_d(g.stretch, sf, 1000).quotient;
        g.shrink = xn_over_d(g.shrink, 1000, sf).quotient;
        Node::Glue(GlueNode::new(g))
    };
    tail_append(eng, node);
}

/// `hmode+spacer` (§1041).
pub(crate) fn append_space(eng: &mut Eng) {
    if eng.aux.space_factor == 1000 || abs_mode(eng) != super::HMODE {
        append_normal_space(eng);
    } else {
        app_space(eng);
    }
}

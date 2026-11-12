//! Hyphenation: patterns and exceptions, finding hyphens and reconstituting words (§891–§965).

use std::collections::{HashMap, VecDeque};

use rtex_core::Res;
use rtex_core::cmd;

use super::Eng;
use super::fonts::new_character;
use super::node::{DiscNode, KernKind, LigatureNode, Node, Whatsit};

const NON_CHAR: u32 = 256;
const UC_HYPH: usize = 38;
const LANGUAGE: usize = 50;

/// Patterns and exceptions for every language.
#[derive(Clone, Debug, Default)]
pub(crate) struct Hyphenation {
    patterns: HashMap<Vec<u32>, Option<Vec<u8>>>,
    languages: std::collections::HashSet<u32>,
    exceptions: HashMap<(u32, Vec<u32>), Vec<usize>>,
    /// Whether the patterns are frozen (TeX's `trie_not_ready` is false).
    pub trie_ready: bool,
}

impl Hyphenation {
    /// Serializes the patterns, languages and exceptions for a format image.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.trie_ready as u8);
        let mut patterns: Vec<(&Vec<u32>, &Option<Vec<u8>>)> = self.patterns.iter().collect();
        patterns.sort();
        put_u32(&mut out, patterns.len() as u32);
        for (key, digits) in patterns {
            put_seq(&mut out, key);
            match digits {
                Some(d) => {
                    out.push(1);
                    put_u32(&mut out, d.len() as u32);
                    out.extend_from_slice(d);
                }
                None => out.push(0),
            }
        }
        let mut languages: Vec<u32> = self.languages.iter().copied().collect();
        languages.sort_unstable();
        put_u32(&mut out, languages.len() as u32);
        for lang in languages {
            put_u32(&mut out, lang);
        }
        let mut exceptions: Vec<(ExceptionKey<'_>, &Vec<usize>)> = self.exceptions.iter().collect();
        exceptions.sort();
        put_u32(&mut out, exceptions.len() as u32);
        for ((lang, word), positions) in exceptions {
            put_u32(&mut out, *lang);
            put_seq(&mut out, word);
            put_u32(&mut out, positions.len() as u32);
            for &position in positions {
                put_u32(&mut out, position as u32);
            }
        }
        out
    }

    /// Restores a format image written by [`Hyphenation::encode`].
    pub(crate) fn decode(&mut self, data: &[u8]) -> bool {
        let mut reader = Reader { data, at: 0 };
        let Some(ready) = reader.u8() else {
            return false;
        };
        let Some(pattern_count) = reader.u32() else {
            return false;
        };
        let mut patterns = HashMap::new();
        for _ in 0..pattern_count as usize {
            let Some(key) = reader.seq() else {
                return false;
            };
            let Some(present) = reader.u8() else {
                return false;
            };
            let digits = if present != 0 {
                match reader.seq_u8() {
                    Some(d) => Some(d),
                    None => return false,
                }
            } else {
                None
            };
            patterns.insert(key, digits);
        }
        let Some(language_count) = reader.u32() else {
            return false;
        };
        let mut languages = std::collections::HashSet::new();
        for _ in 0..language_count as usize {
            let Some(lang) = reader.u32() else {
                return false;
            };
            languages.insert(lang);
        }
        let Some(exception_count) = reader.u32() else {
            return false;
        };
        let mut exceptions = HashMap::new();
        for _ in 0..exception_count as usize {
            let Some(lang) = reader.u32() else {
                return false;
            };
            let Some(word) = reader.seq() else {
                return false;
            };
            let Some(position_count) = reader.u32() else {
                return false;
            };
            let mut positions = Vec::with_capacity(position_count as usize);
            for _ in 0..position_count as usize {
                let Some(position) = reader.u32() else {
                    return false;
                };
                positions.push(position as usize);
            }
            exceptions.insert((lang, word), positions);
        }
        self.patterns = patterns;
        self.languages = languages;
        self.exceptions = exceptions;
        self.trie_ready = ready != 0;
        true
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_seq(out: &mut Vec<u8>, values: &[u32]) {
    put_u32(out, values.len() as u32);
    for &value in values {
        put_u32(out, value);
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let value = *self.data.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes = self.data.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let bytes = self.data.get(self.at..self.at + len)?;
        self.at += len;
        Some(bytes)
    }

    fn seq(&mut self) -> Option<Vec<u32>> {
        let len = self.u32()? as usize;
        let mut values = Vec::with_capacity(len);
        for _ in 0..len {
            values.push(self.u32()?);
        }
        Some(values)
    }

    fn seq_u8(&mut self) -> Option<Vec<u8>> {
        let len = self.u32()? as usize;
        self.bytes(len).map(<[u8]>::to_vec)
    }
}

type ExceptionKey<'a> = &'a (u32, Vec<u32>);

/// The language and hyphen-min values in effect while scanning a paragraph.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HyphState {
    pub cur_lang: i32,
    pub l_hyf: i32,
    pub r_hyf: i32,
}

fn lang_of(eng: &Eng) -> u32 {
    let l = eng.int_par(LANGUAGE);
    if (1..=255).contains(&l) { l as u32 } else { 0 }
}

/// `new_patterns` (§960).
pub(crate) fn new_patterns(eng: &mut Eng) -> Res {
    if !eng.is_initex() {
        eng.print_err("Patterns can be loaded only by INITEX");
        eng.set_help(&[]);
        eng.error()?;
        eng.scan_token_list(false)?;
        return Ok(());
    }
    if eng.stomach.hyph.trie_ready {
        eng.print_err("Too late for ");
        eng.print_esc("patterns");
        eng.set_help(&["All patterns must be given before typesetting begins."]);
        eng.error()?;
        eng.scan_token_list(false)?;
        return Ok(());
    }
    let lang = lang_of(eng);
    eng.scan_left_brace()?;
    let mut hc: Vec<u32> = Vec::new();
    let mut hyf: Vec<u8> = vec![0];
    let mut digit_sensed = false;
    loop {
        eng.get_x_token()?;
        match eng.cur_cmd() {
            cmd::LETTER | cmd::OTHER_CHAR => {
                let c = eng.cur_chr();
                if digit_sensed || !(u32::from('0')..=u32::from('9')).contains(&c) {
                    let c = if c == u32::from('.') {
                        0
                    } else {
                        let l = eng.eqtb().lc_code(c);
                        if l == 0 {
                            eng.print_err("Nonletter");
                            eng.set_help(&["(See Appendix H.)"]);
                            eng.error()?;
                        }
                        l
                    };
                    if hc.len() < 63 {
                        hc.push(c);
                        hyf.push(0);
                        digit_sensed = false;
                    }
                } else if hc.len() < 63 {
                    *hyf.last_mut().expect("digits") = (c - u32::from('0')) as u8;
                    digit_sensed = true;
                }
            }
            cmd::SPACER | cmd::RIGHT_BRACE => {
                if !hc.is_empty() {
                    insert_pattern(eng, lang, &hc, &mut hyf)?;
                }
                if eng.cur_cmd() == cmd::RIGHT_BRACE {
                    return Ok(());
                }
                hc.clear();
                hyf = vec![0];
                digit_sensed = false;
            }
            _ => {
                eng.print_err("Bad ");
                eng.print_esc("patterns");
                eng.set_help(&["(See Appendix H.)"]);
                eng.error()?;
            }
        }
    }
}

fn insert_pattern(eng: &mut Eng, lang: u32, hc: &[u32], hyf: &mut [u8]) -> Res {
    let k = hc.len();
    if hc[0] == 0 {
        hyf[0] = 0;
    }
    if hc[k - 1] == 0 {
        hyf[k] = 0;
    }
    let mut key = vec![lang];
    let h = &mut eng.stomach.hyph;
    h.languages.insert(lang);
    for &c in hc {
        key.push(c);
        h.patterns.entry(key.clone()).or_insert(None);
    }
    let has_digits = hyf.iter().any(|&d| d != 0);
    let slot = h.patterns.get_mut(&key).expect("pattern slot");
    if slot.is_some() {
        eng.print_err("Duplicate pattern");
        eng.set_help(&["(See Appendix H.)"]);
        return eng.error();
    }
    if has_digits {
        *slot = Some(hyf.to_vec());
    }
    Ok(())
}

/// `new_hyph_exceptions` (§934).
pub(crate) fn new_hyph_exceptions(eng: &mut Eng) -> Res {
    eng.scan_left_brace()?;
    let lang = lang_of(eng);
    let mut word: Vec<u32> = Vec::new();
    let mut hyphens: Vec<usize> = Vec::new();
    loop {
        eng.get_x_token()?;
        let mut cmd = eng.cur_cmd();
        let mut chr = eng.cur_chr();
        if cmd == cmd::CHAR_NUM {
            chr = eng.scan_char_num()? as u32;
            cmd = cmd::CHAR_GIVEN;
        }
        match cmd {
            cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN => {
                if chr == u32::from('-') {
                    if word.len() < 63 {
                        hyphens.push(word.len());
                    }
                } else {
                    let lc = eng.eqtb().lc_code(chr);
                    if lc == 0 {
                        eng.print_err("Not a letter");
                        eng.set_help(&[
                            "Letters in \\hyphenation words must have \\lccode>0.",
                            "Proceed; I'll ignore the character I just read.",
                        ]);
                        eng.error()?;
                    } else if word.len() < 63 {
                        word.push(lc);
                    }
                }
            }
            cmd::SPACER | cmd::RIGHT_BRACE => {
                if word.len() > 1 {
                    eng.stomach
                        .hyph
                        .exceptions
                        .insert((lang, std::mem::take(&mut word)), std::mem::take(&mut hyphens));
                }
                if cmd == cmd::RIGHT_BRACE {
                    return Ok(());
                }
                word.clear();
                hyphens.clear();
            }
            _ => {
                eng.print_err("Improper ");
                eng.print_esc("hyphenation");
                eng.print_str(" will be flushed");
                eng.set_help(&[
                    "Hyphenation exceptions must contain only letters",
                    "and hyphens. But continue; I'll forgive and forget.",
                ]);
                eng.error()?;
            }
        }
    }
}

struct Word {
    hf: usize,
    hu: [u32; 66],
    hc: [u32; 66],
    hyf: [u8; 66],
    hn: usize,
    hyf_char: u32,
    hyf_bchar: u32,
    init_list: Vec<u32>,
    init_lig: bool,
    init_lft: bool,
    hyphen_passed: usize,
}

/// "Try to hyphenate the following word" (§894): `cur_p` is a glue node in `list`.
pub(crate) fn try_hyphenate(eng: &mut Eng, list: &mut Vec<Node>, cur_p: usize, st: &mut HyphState) -> Res {
    let lc = |eng: &Eng, c: u32| eng.eqtb().lc_code(c);
    let mut prev_s = cur_p;
    let mut s = cur_p + 1;
    let (hf, _c) = loop {
        let Some(node) = list.get(s) else { return Ok(()) };
        let (c, f) = match node {
            Node::Char { font, ch } => (*ch, *font),
            Node::Ligature(l) => match l.original.first() {
                None => {
                    prev_s = s;
                    s += 1;
                    continue;
                }
                Some(&c) => (c, l.font),
            },
            Node::Kern {
                kind: KernKind::Normal, ..
            } => {
                prev_s = s;
                s += 1;
                continue;
            }
            Node::Whatsit(w) => {
                if let Whatsit::Language { lang, lhm, rhm } = w {
                    st.cur_lang = *lang;
                    st.l_hyf = *lhm;
                    st.r_hyf = *rhm;
                }
                prev_s = s;
                s += 1;
                continue;
            }
            _ => return Ok(()),
        };
        let l = lc(eng, c);
        if l != 0 {
            if l == c || eng.int_par(UC_HYPH) > 0 {
                break (f, c);
            }
            return Ok(());
        }
        prev_s = s;
        s += 1;
    };
    let hyf_char = eng.fonts().get(hf).map_or(-1, |i| i.hyphen_char);
    if !(0..=255).contains(&hyf_char) {
        return Ok(());
    }
    let ha = prev_s;
    if st.l_hyf + st.r_hyf > 63 {
        return Ok(());
    }
    let font_bchar = eng
        .stomach
        .fonts
        .get(hf)
        .and_then(|t| t.boundary_char())
        .map_or(NON_CHAR, u32::from);
    let mut w = Word {
        hf,
        hu: [0; 66],
        hc: [0; 66],
        hyf: [0; 66],
        hn: 0,
        hyf_char: hyf_char as u32,
        hyf_bchar: NON_CHAR,
        init_list: Vec::new(),
        init_lig: false,
        init_lft: false,
        hyphen_passed: 0,
    };
    let mut hb = None;
    while let Some(node) = list.get(s) {
        match node {
            Node::Char { font, ch } => {
                if *font != hf {
                    break;
                }
                w.hyf_bchar = *ch;
                let l = lc(eng, *ch);
                if l == 0 || w.hn == 63 {
                    break;
                }
                hb = Some(s);
                w.hn += 1;
                w.hu[w.hn] = *ch;
                w.hc[w.hn] = l;
                w.hyf_bchar = NON_CHAR;
            }
            Node::Ligature(lig) => {
                if lig.font != hf {
                    break;
                }
                let mut j = w.hn;
                if let Some(&c) = lig.original.first() {
                    w.hyf_bchar = c;
                }
                let mut ok = true;
                for &c in &lig.original {
                    let l = lc(eng, c);
                    if l == 0 || j == 63 {
                        ok = false;
                        break;
                    }
                    j += 1;
                    w.hu[j] = c;
                    w.hc[j] = l;
                }
                if !ok {
                    break;
                }
                hb = Some(s);
                w.hn = j;
                w.hyf_bchar = if lig.right_hit { font_bchar } else { NON_CHAR };
            }
            Node::Kern {
                kind: KernKind::Normal, ..
            } => {
                hb = Some(s);
                w.hyf_bchar = font_bchar;
            }
            _ => break,
        }
        s += 1;
    }
    if (w.hn as i32) < st.l_hyf + st.r_hyf {
        return Ok(());
    }
    loop {
        match list.get(s) {
            None => break,
            Some(Node::Char { .. } | Node::Ligature(_)) => {}
            Some(Node::Kern { kind, .. }) => {
                if *kind != KernKind::Normal {
                    break;
                }
            }
            Some(
                Node::Whatsit(_) | Node::Glue(_) | Node::Penalty(_) | Node::Ins(_) | Node::Adjust(_) | Node::Mark(_),
            ) => break,
            Some(_) => return Ok(()),
        }
        s += 1;
    }
    let Some(hb) = hb else { return Ok(()) };
    if !find_hyphens(eng, &mut w, st) {
        return Ok(());
    }
    replace_word(eng, list, cur_p, ha, hb, &mut w)
}

fn find_hyphens(eng: &Eng, w: &mut Word, st: &HyphState) -> bool {
    let hn = w.hn;
    let lang = st.cur_lang.max(0) as u32;
    w.hyf[..=hn].fill(0);
    let h = &eng.stomach.hyph;
    let word: Vec<u32> = w.hc[1..=hn].to_vec();
    if let Some(positions) = h.exceptions.get(&(lang, word)) {
        for &p in positions {
            if p <= hn {
                w.hyf[p] = 1;
            }
        }
    } else {
        if !h.languages.contains(&lang) {
            return false;
        }
        w.hc[0] = 0;
        w.hc[hn + 1] = 0;
        w.hc[hn + 2] = 256;
        let r_hyf = st.r_hyf.max(0) as usize;
        let last = (hn + 1).saturating_sub(r_hyf);
        for j in 0..=last {
            let mut key = vec![lang];
            let mut l = j;
            loop {
                if w.hc[l] == 256 {
                    break;
                }
                key.push(w.hc[l]);
                match h.patterns.get(&key) {
                    None => break,
                    Some(Some(digits)) => {
                        for (i, &d) in digits.iter().enumerate() {
                            if d != 0 {
                                let idx = j + i;
                                if idx >= 1 && w.hyf[idx - 1] < d {
                                    w.hyf[idx - 1] = d;
                                }
                            }
                        }
                    }
                    Some(None) => {}
                }
                l += 1;
            }
        }
    }
    for j in 0..st.l_hyf.max(0) as usize {
        if j <= hn {
            w.hyf[j] = 0;
        }
    }
    for j in 0..st.r_hyf.max(0) as usize {
        if j <= hn {
            w.hyf[hn - j] = 0;
        }
    }
    let lo = st.l_hyf.max(0) as usize;
    let hi = hn as i64 - i64::from(st.r_hyf);
    (lo as i64..=hi).any(|j| j >= 0 && w.hyf[j as usize] % 2 == 1)
}

#[derive(Clone, Copy)]
struct Pseudo {
    ch: u32,
    ptr: Option<u32>,
}

impl Word {
    fn set_cur_r(&self, j: usize, n: usize, bchar: u32, hchar: u32, cur_r: &mut u32, cur_rh: &mut u32) {
        *cur_r = if j < n { self.hu[j + 1] } else { bchar };
        *cur_rh = if self.hyf[j] % 2 == 1 { hchar } else { NON_CHAR };
    }

    #[allow(clippy::too_many_lines)]
    fn reconstitute(
        &mut self,
        eng: &Eng,
        mut j: usize,
        n: usize,
        mut bchar: u32,
        mut hchar: u32,
    ) -> (usize, Vec<Node>) {
        let hf = self.hf;
        let font = eng.stomach.fonts.get(hf);
        self.hyphen_passed = 0;
        let mut t: Vec<Node> = Vec::new();
        let mut w = 0;
        let mut cur_l = self.hu[j];
        let mut cur_q = 0usize;
        let mut ligature_present = false;
        let mut lft_hit = false;
        let mut rt_hit = false;
        if j == 0 {
            ligature_present = self.init_lig;
            if ligature_present {
                lft_hit = self.init_lft;
            }
            for &c in &self.init_list {
                t.push(Node::Char { font: hf, ch: c });
            }
        } else if cur_l < NON_CHAR {
            t.push(Node::Char { font: hf, ch: cur_l });
        }
        let mut lig_stack: VecDeque<Pseudo> = VecDeque::new();
        let (mut cur_r, mut cur_rh) = (0, 0);
        self.set_cur_r(j, n, bchar, hchar, &mut cur_r, &mut cur_rh);
        let wrap_lig = |t: &mut Vec<Node>,
                        cur_q: usize,
                        cur_l: u32,
                        ligature_present: &mut bool,
                        lft_hit: &mut bool,
                        rt_hit: &mut bool,
                        rt: bool,
                        stack_empty: bool| {
            if *ligature_present {
                let original: Vec<u32> = t
                    .drain(cur_q..)
                    .filter_map(|n| match n {
                        Node::Char { ch, .. } => Some(ch),
                        _ => None,
                    })
                    .collect();
                let mut lig = LigatureNode {
                    font: hf,
                    ch: cur_l,
                    original,
                    left_hit: false,
                    right_hit: false,
                };
                if *lft_hit {
                    lig.left_hit = true;
                    *lft_hit = false;
                }
                if rt && stack_empty {
                    lig.right_hit = true;
                    *rt_hit = false;
                }
                t.push(Node::Ligature(lig));
                *ligature_present = false;
            }
        };
        'cont: loop {
            let start = match font {
                Some(f) if cur_l == NON_CHAR => f.bchar_label(),
                Some(f) => f.program_of(cur_l),
                None => None,
            };
            'done: {
                let Some(mut k) = start else { break 'done };
                let font = font.expect("font with a program");
                let test_char = if cur_rh < NON_CHAR { cur_rh } else { cur_r };
                loop {
                    let Some(q) = font.step(k) else { break 'done };
                    if u32::from(q.next_char) == test_char && q.skip <= 128 {
                        if cur_rh < NON_CHAR {
                            self.hyphen_passed = j;
                            hchar = NON_CHAR;
                            cur_rh = NON_CHAR;
                            continue 'cont;
                        }
                        if hchar < NON_CHAR && self.hyf[j] % 2 == 1 {
                            self.hyphen_passed = j;
                            hchar = NON_CHAR;
                        }
                        if !q.is_kern() {
                            if cur_l == NON_CHAR {
                                lft_hit = true;
                            }
                            if j == n && lig_stack.is_empty() {
                                rt_hit = true;
                            }
                            let rem = u32::from(q.remainder);
                            match q.op {
                                1 | 5 => {
                                    cur_l = rem;
                                    ligature_present = true;
                                }
                                2 | 6 => {
                                    cur_r = rem;
                                    if let Some(front) = lig_stack.front_mut() {
                                        front.ch = rem;
                                    } else {
                                        let ptr = if j == n {
                                            bchar = NON_CHAR;
                                            None
                                        } else {
                                            Some(self.hu[j + 1])
                                        };
                                        lig_stack.push_front(Pseudo { ch: rem, ptr });
                                    }
                                }
                                3 => {
                                    cur_r = rem;
                                    lig_stack.push_front(Pseudo { ch: rem, ptr: None });
                                }
                                7 | 11 => {
                                    let empty = lig_stack.is_empty();
                                    wrap_lig(
                                        &mut t,
                                        cur_q,
                                        cur_l,
                                        &mut ligature_present,
                                        &mut lft_hit,
                                        &mut rt_hit,
                                        false,
                                        empty,
                                    );
                                    cur_q = t.len();
                                    cur_l = rem;
                                    ligature_present = true;
                                }
                                _ => {
                                    cur_l = rem;
                                    ligature_present = true;
                                    if let Some(front) = lig_stack.pop_front() {
                                        if let Some(c) = front.ptr {
                                            t.push(Node::Char { font: hf, ch: c });
                                            j += 1;
                                        }
                                        match lig_stack.front() {
                                            None => self.set_cur_r(j, n, bchar, hchar, &mut cur_r, &mut cur_rh),
                                            Some(next) => cur_r = next.ch,
                                        }
                                    } else if j == n {
                                        break 'done;
                                    } else {
                                        t.push(Node::Char { font: hf, ch: cur_r });
                                        j += 1;
                                        self.set_cur_r(j, n, bchar, hchar, &mut cur_r, &mut cur_rh);
                                    }
                                }
                            }
                            if q.op > 4 && q.op != 7 {
                                break 'done;
                            }
                            continue 'cont;
                        }
                        w = font.kern(&q);
                        break 'done;
                    }
                    if q.skip >= 128 {
                        if cur_rh == NON_CHAR {
                            break 'done;
                        }
                        cur_rh = NON_CHAR;
                        continue 'cont;
                    }
                    k += usize::from(q.skip) + 1;
                }
            }
            let empty = lig_stack.is_empty();
            let rt = rt_hit;
            wrap_lig(
                &mut t,
                cur_q,
                cur_l,
                &mut ligature_present,
                &mut lft_hit,
                &mut rt_hit,
                rt,
                empty,
            );
            if w != 0 {
                t.push(Node::kern(w, KernKind::Normal));
                w = 0;
            }
            if let Some(front) = lig_stack.pop_front() {
                cur_q = t.len();
                cur_l = front.ch;
                ligature_present = true;
                if let Some(c) = front.ptr {
                    t.push(Node::Char { font: hf, ch: c });
                    j += 1;
                }
                match lig_stack.front() {
                    None => self.set_cur_r(j, n, bchar, hchar, &mut cur_r, &mut cur_rh),
                    Some(next) => cur_r = next.ch,
                }
                continue 'cont;
            }
            break;
        }
        (j, t)
    }
}

fn replace_word(eng: &mut Eng, list: &mut Vec<Node>, cur_p: usize, ha: usize, hb: usize, w: &mut Word) -> Res {
    let hf = w.hf;
    let bchar = w.hyf_bchar;
    let font_bchar = eng
        .stomach
        .fonts
        .get(hf)
        .and_then(|t| t.boundary_char())
        .map_or(NON_CHAR, u32::from);
    let has_bchar_label = eng.stomach.fonts.get(hf).and_then(|t| t.bchar_label()).is_some();
    let mut found2 = false;
    let (s, mut j) = match &list[ha] {
        Node::Char { font, ch } if ha != cur_p => {
            if *font != hf {
                found2 = true;
                (ha, 0)
            } else {
                w.init_list = vec![*ch];
                w.init_lig = false;
                w.hu[0] = *ch;
                (ha - 1, 0)
            }
        }
        Node::Ligature(l) if ha != cur_p => {
            if l.font != hf {
                found2 = true;
                (ha, 0)
            } else {
                w.init_list = l.original.clone();
                w.init_lig = true;
                w.init_lft = l.left_hit;
                w.hu[0] = l.ch;
                if w.init_list.is_empty() && w.init_lft {
                    w.hu[0] = 256;
                    w.init_lig = false;
                }
                (ha - 1, 0)
            }
        }
        _ => {
            if let Node::Ligature(l) = &list[ha + 1]
                && l.left_hit
            {
                found2 = true;
            }
            if found2 {
                (ha, 0)
            } else {
                w.init_list.clear();
                (ha, 1)
            }
        }
    };
    if found2 {
        w.hu[0] = 256;
        w.init_lig = false;
        w.init_list.clear();
    }
    let hn = w.hn;
    let hyf_char = w.hyf_char;
    let mut out: Vec<Node> = Vec::new();
    loop {
        let mut l = j;
        let (jr, hold) = w.reconstitute(eng, j, hn, bchar, hyf_char);
        j = jr + 1;
        let mut hold = hold;
        if w.hyphen_passed == 0 {
            out.append(&mut hold);
            if w.hyf[j - 1] % 2 == 1 {
                l = j;
                w.hyphen_passed = j - 1;
            }
        }
        if w.hyphen_passed > 0 {
            loop {
                let mut major = std::mem::take(&mut hold);
                let mut i = w.hyphen_passed;
                w.hyf[i] = 0;
                let mut pre = Vec::new();
                let hyf_node = new_character(eng, hf, hyf_char).is_some();
                let mut saved = 0;
                if hyf_node {
                    i += 1;
                    saved = w.hu[i];
                    w.hu[i] = hyf_char;
                }
                while l <= i {
                    let (lj, h) = w.reconstitute(eng, l, i, font_bchar, NON_CHAR);
                    l = lj + 1;
                    pre.extend(h);
                }
                if hyf_node {
                    w.hu[i] = saved;
                    l = i;
                    i -= 1;
                }
                let _ = i;
                let mut post = Vec::new();
                let mut c_loc = 0;
                let mut c = 0;
                if has_bchar_label {
                    l -= 1;
                    c = w.hu[l];
                    c_loc = l;
                    w.hu[l] = 256;
                }
                while l < j {
                    loop {
                        let (lj, h) = w.reconstitute(eng, l, hn, bchar, NON_CHAR);
                        l = lj + 1;
                        if c_loc > 0 {
                            w.hu[c_loc] = c;
                            c_loc = 0;
                        }
                        post.extend(h);
                        if l >= j {
                            break;
                        }
                    }
                    while l > j {
                        let (jj, h) = w.reconstitute(eng, j, hn, bchar, NON_CHAR);
                        j = jj + 1;
                        major.extend(h);
                    }
                }
                if major.len() > 127 {
                    out.extend(major);
                } else {
                    out.push(Node::Disc(Box::new(DiscNode {
                        pre,
                        post,
                        replace_count: major.len(),
                    })));
                    out.extend(major);
                }
                w.hyphen_passed = j - 1;
                hold = Vec::new();
                if w.hyf[j - 1].is_multiple_of(2) {
                    break;
                }
            }
        }
        if j > hn {
            break;
        }
    }
    list.splice(s + 1..=hb, out);
    Ok(())
}

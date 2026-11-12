//! `ship_out` (§592–§640): turning a finished box into a positioned page.

use std::sync::Arc;

use rtex_core::{Res, Token};

use super::Eng;
use super::backend::{Action, ActionTarget, AltRule, BackendItem, ColorOp, DestKind, Ident, LiteralMode};
use super::node::{BoxDir, BoxNode, GlueKind, GlueSign, MAX_DIMEN, Node, RUNNING, Whatsit};
use super::pdftex::{dimen, int, pdf_dimen, pdf_error, pdf_int, pdf_warning};
use super::show::show_box_node;
use crate::document::{Area, DestName, DocFont, DrawOp, FontMetrics, Page, PdfAction, PdfOp, PdfTarget};
use crate::units::Scaled;

const TRACING_OUTPUT: usize = 34;
const HOFFSET: usize = 18;
const VOFFSET: usize = 19;
const BILLION: f64 = 1_000_000_000.0;

/// The box being output, for whatsits whose dimensions run to its edges.
#[derive(Clone, Copy)]
struct Parent {
    left: Scaled,
    base: Scaled,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
}

/// A link annotation not yet closed by `\pdfendlink` (pdfTeX's `pdf_link_stack`).
#[derive(Clone)]
struct OpenLink {
    obj: usize,
    rule: AltRule,
    attr: Option<String>,
    action: PdfAction,
    left: Scaled,
    top: Scaled,
    bottom: Scaled,
    level: usize,
}

struct Out<'a> {
    eng: &'a mut Eng,
    ops: Vec<DrawOp>,
    doing_leaders: bool,
    run_end: Option<(usize, Scaled, Scaled)>,
    page_height: Scaled,
    level: usize,
    parents: Vec<Parent>,
    links: Vec<OpenLink>,
    saves: Vec<(Scaled, Scaled)>,
    shipping_page: bool,
}

fn vet(g: f64) -> f64 {
    g.clamp(-BILLION, BILLION)
}

impl<'a> Out<'a> {
    /// Executes the packet of character `ch` of virtual font `font` (`do_vf_packet` in
    /// pdftex.web): emits the packet-local fonts' glyphs with the kerns applied, returning
    /// the total advance. `depth` bounds packet recursion as pdfTeX's `vf_max_recursion` does.
    fn virtual_glyph(&mut self, font: usize, ch: u32, h: Scaled, v: Scaled, depth: u32) -> Scaled {
        const MAX_VF_RECURSION: u32 = 10;
        if depth >= MAX_VF_RECURSION {
            return 0;
        }
        let (local_fonts, ops) = {
            let Some(vf) = self.eng.stomach.fonts.virtuals(font) else {
                return 0;
            };
            match vf.packets.get(&ch) {
                Some(p) => (vf.local_fonts.clone(), p.clone()),
                None => {
                    super::fonts::char_warning(self.eng, font, ch);
                    return 0;
                }
            }
        };
        let mut pen = h;
        for op in ops {
            match op {
                super::vfont::VfOp::Kern(w) => pen += w,
                super::vfont::VfOp::SetChar { local, ch } => {
                    let f = local_fonts.get((local - 1) as usize).copied().unwrap_or(0);
                    pen += self.glyph_at(f, ch, pen, v, depth + 1);
                }
            }
        }
        pen - h
    }

    fn glyph(&mut self, font: usize, ch: u32, h: Scaled, v: Scaled) -> Scaled {
        self.glyph_at(font, ch, h, v, 0)
    }

    fn glyph_at(&mut self, font: usize, ch: u32, h: Scaled, v: Scaled, depth: u32) -> Scaled {
        if self.eng.stomach.fonts.virtuals(font).is_some() {
            return self.virtual_glyph(font, ch, h, v, depth);
        }
        let w = self.eng.stomach.fonts.width(font, ch);
        if self.eng.stomach.document.font(font).is_none()
            && let Some(t) = self.eng.stomach.fonts.get(font)
        {
            let metrics = FontMetrics {
                ascent: t.height(u32::from('h')),
                cap_height: t.height(u32::from('H')),
                descent: t.depth(u32::from('y')),
                x_height: super::fonts::param(self.eng, font, 4),
            };
            let doc = DocFont {
                name: self.eng.stomach.fonts.name(font).to_owned(),
                size: t.size,
                design_size: t.design_size,
                checksum: t.checksum,
                metrics,
            };
            let name = doc.name.clone();
            self.eng.stomach.document.set_font(font, doc);
            super::pdftex::init_font(self.eng, &name);
        }
        let continues = self.run_end == Some((font, h, v));
        match self.ops.last_mut() {
            Some(DrawOp::Text { codes, advances, .. }) if continues => {
                codes.push(ch);
                advances.push(w);
            }
            _ => self.ops.push(DrawOp::Text {
                font,
                x: h,
                baseline: v,
                codes: vec![ch],
                advances: vec![w],
            }),
        }
        self.run_end = Some((font, h + w, v));
        w
    }

    fn rule(&mut self, x: Scaled, top: Scaled, width: Scaled, height: Scaled) {
        if width > 0 && height > 0 {
            self.ops.push(DrawOp::Rule { x, top, width, height });
        }
        self.run_end = None;
    }

    fn new(eng: &'a mut Eng, page_height: Scaled, shipping_page: bool) -> Out<'a> {
        Out {
            eng,
            ops: Vec::new(),
            doing_leaders: false,
            run_end: None,
            page_height,
            level: 0,
            parents: Vec::new(),
            links: Vec::new(),
            saves: Vec::new(),
            shipping_page,
        }
    }

    fn pdf(&mut self, x: Scaled, y: Scaled, op: PdfOp) {
        self.run_end = None;
        self.ops.push(DrawOp::Pdf { x, y, op });
    }

    fn literal(&mut self, x: Scaled, y: Scaled, mode: LiteralMode, text: String) {
        self.pdf(x, y, PdfOp::Literal { mode, text });
    }

    fn string(&mut self, toks: &[Token]) -> String {
        self.eng.token_list_string(toks)
    }

    fn parent(&self) -> Parent {
        self.parents.last().copied().unwrap_or(Parent {
            left: 0,
            base: 0,
            width: 0,
            height: 0,
            depth: 0,
        })
    }

    /// `set_rect_dimens`.
    fn rect(&self, rule: &AltRule, h: Scaled, v: Scaled, margin: Scaled) -> Area {
        let p = self.parent();
        Area {
            left: h - margin,
            right: if rule.width == RUNNING {
                p.left + p.width
            } else {
                h + rule.width
            } + margin,
            top: if rule.height == RUNNING {
                p.base - p.height
            } else {
                v - rule.height
            } - margin,
            bottom: if rule.depth == RUNNING {
                p.base + p.depth
            } else {
                v + rule.depth
            } + margin,
        }
    }

    fn action(&mut self, a: &Action) -> PdfAction {
        resolve_action(self.eng, a)
    }

    fn special(&mut self, h: Scaled, v: Scaled, text: String) {
        if let Some(rest) = text.strip_prefix("rtex:") {
            self.pdf(h, v, PdfOp::Structure(rest.to_owned()));
            return;
        }
        let Some(rest) = text.strip_prefix("PDF:").or_else(|| text.strip_prefix("pdf:")) else {
            if pdf_int(self.eng, int::OUTPUT) > 0 && !text.starts_with("SRC:") && !text.starts_with("src:") {
                pdf_warning(self.eng, "\\special", "non-PDF special ignored!", true, true);
            }
            self.ops.push(DrawOp::Special { x: h, y: v, text });
            return;
        };
        let (mode, code) = if let Some(c) = rest.strip_prefix("direct:") {
            (LiteralMode::Direct, c)
        } else if let Some(c) = rest.strip_prefix("page:") {
            (LiteralMode::Page, c)
        } else {
            (LiteralMode::SetOrigin, rest)
        };
        self.literal(h, v, mode, code.to_owned());
    }

    fn color_stack(&mut self, h: Scaled, v: Scaled, stack: usize, op: ColorOp, toks: Option<&Arc<[Token]>>) {
        let data = toks.map(|t| self.string(t));
        let Some(s) = self.eng.stomach.backend.color_stacks.get_mut(stack) else {
            self.eng.print_nl("");
            self.eng.print_str("Color stack ");
            self.eng.print_int(stack as i32);
            self.eng.print_str(" is not initialized for use!");
            self.eng.print_nl("");
            return;
        };
        let mode = s.mode;
        let out = match op {
            ColorOp::Set => {
                let d = data.unwrap_or_default();
                if let Some(top) = s.values.last_mut() {
                    *top = d.clone();
                }
                d
            }
            ColorOp::Push => {
                let d = data.unwrap_or_default();
                s.values.push(d.clone());
                d
            }
            ColorOp::Pop => {
                if s.values.len() > 1 {
                    s.values.pop();
                } else {
                    pdf_warning(self.eng, "pdfcolorstack", "pop empty color page stack 0", true, true);
                }
                let s = &self.eng.stomach.backend.color_stacks[stack];
                s.values.last().cloned().unwrap_or_default()
            }
            ColorOp::Current => s.values.last().cloned().unwrap_or_default(),
        };
        if !out.is_empty() {
            self.literal(h, v, mode, out);
        }
    }

    fn start_link(&mut self, obj: usize, rule: AltRule, attr: Option<String>, action: PdfAction, h: Scaled, v: Scaled) {
        let r = self.rect(&rule, h, v, 0);
        self.links.push(OpenLink {
            obj,
            rule,
            attr,
            action,
            left: h,
            top: r.top,
            bottom: r.bottom,
            level: self.level,
        });
    }

    fn close_link(&mut self, l: &OpenLink, right: Scaled) {
        let m = pdf_dimen(self.eng, dimen::LINK_MARGIN);
        let area = Area {
            left: l.left - m,
            top: l.top - m,
            right: right + m,
            bottom: l.bottom + m,
        };
        self.pdf(
            l.left,
            l.top,
            PdfOp::Link {
                obj: l.obj,
                area,
                attr: l.attr.clone(),
                action: l.action.clone(),
            },
        );
    }

    fn dest(&mut self, id: &Ident, kind: DestKind, rule: &AltRule, h: Scaled, v: Scaled) {
        let name = match id {
            Ident::Name(t) => DestName::Name(self.string(t)),
            Ident::Num(n) => DestName::Num(*n),
        };
        let area = if kind == DestKind::FitR {
            let m = pdf_dimen(self.eng, dimen::DEST_MARGIN);
            self.rect(rule, h, v, m)
        } else {
            Area {
                left: h,
                top: v,
                right: h,
                bottom: v,
            }
        };
        self.pdf(h, v, PdfOp::Dest { name, kind, area });
    }

    fn backend(&mut self, item: &BackendItem, h: Scaled, v: Scaled) -> Res {
        match item {
            BackendItem::Literal { mode, late, toks } => {
                let text = if *late {
                    let t = self.eng.expand_write_text(toks.clone())?;
                    self.string(&t)
                } else {
                    self.string(toks)
                };
                self.literal(h, v, *mode, text);
            }
            BackendItem::LateSpecial(toks) => {
                let t = self.eng.expand_write_text(toks.clone())?;
                let text = self.string(&t);
                self.special(h, v, text);
            }
            BackendItem::ColorStack { stack, op, toks } => self.color_stack(h, v, *stack, *op, toks.as_ref()),
            BackendItem::SetMatrix(toks) => {
                let text = self.string(toks);
                let ok = text.split_whitespace().filter(|w| w.parse::<f64>().is_ok()).count() == 4
                    && text.split_whitespace().count() == 4;
                if !ok {
                    return pdf_error(self.eng, "\\pdfsetmatrix", "Unrecognized format.");
                }
                self.literal(h, v, LiteralMode::SetOrigin, format!("{text} 0 0 cm"));
            }
            BackendItem::Save => {
                self.saves.push((h, v));
                self.literal(h, v, LiteralMode::SetOrigin, "q".to_owned());
            }
            BackendItem::Restore => {
                match self.saves.pop() {
                    Some((sh, sv)) if sh != h || sv != v => {
                        pdf_warning(self.eng, "", "Misplaced \\pdfrestore by (", true, false);
                        self.eng.print_scaled(h - sh);
                        self.eng.print_str("sp, ");
                        self.eng.print_scaled(sv - v);
                        self.eng.print_str("sp)");
                        self.eng.print_ln();
                    }
                    Some(_) => {}
                    None => pdf_warning(self.eng, "", "\\pdfrestore: missing \\pdfsave", true, true),
                }
                self.literal(h, v, LiteralMode::SetOrigin, "Q".to_owned());
            }
            BackendItem::RefObj(_) | BackendItem::RefXForm(_) | BackendItem::RefXImage(_) => {}
            BackendItem::Annot { obj, rule, toks } => {
                if !self.shipping_page {
                    return pdf_error(self.eng, "ext4", "annotations cannot be inside an XForm");
                }
                if self.doing_leaders {
                    return Ok(());
                }
                let area = self.rect(rule, h, v, 0);
                let data = self.string(toks);
                self.pdf(h, v, PdfOp::Annot { obj: *obj, area, data });
            }
            BackendItem::StartLink {
                obj,
                rule,
                attr,
                action,
            } => {
                if !self.shipping_page {
                    return pdf_error(self.eng, "ext4", "link annotations cannot be inside an XForm");
                }
                if self.doing_leaders {
                    return Ok(());
                }
                let attr = attr.as_ref().map(|a| self.eng.token_list_string(a));
                let action = self.action(action);
                self.start_link(*obj, *rule, attr, action, h, v);
            }
            BackendItem::EndLink => {
                if self.doing_leaders {
                    return Ok(());
                }
                let Some(l) = self.links.pop() else {
                    return pdf_error(
                        self.eng,
                        "ext4",
                        "pdf_link_stack empty, \\pdfendlink used without \\pdfstartlink?",
                    );
                };
                if l.level != self.level {
                    return pdf_error(
                        self.eng,
                        "ext4",
                        "\\pdfendlink ended up in different nesting level than \\pdfstartlink",
                    );
                }
                let right = if l.rule.width == RUNNING {
                    h
                } else {
                    l.left + l.rule.width
                };
                self.close_link(&l, right);
            }
            BackendItem::Dest { id, kind, rule, .. } => {
                if !self.shipping_page {
                    return pdf_error(self.eng, "ext4", "destinations cannot be inside an XForm");
                }
                if !self.doing_leaders && super::pdftex::ship_dest(self.eng, id) {
                    self.dest(id, *kind, rule, h, v);
                }
            }
            BackendItem::SavePos => {
                self.eng.stomach.backend.last_x_pos = h;
                self.eng.stomach.backend.last_y_pos = self.page_height - v;
            }
            BackendItem::Structure(t) => self.pdf(h, v, PdfOp::Structure(t.to_string())),
            BackendItem::Thread { .. }
            | BackendItem::EndThread
            | BackendItem::SnapRefPoint
            | BackendItem::SnapY(..)
            | BackendItem::SnapYComp(_)
            | BackendItem::InterwordSpaceOn
            | BackendItem::InterwordSpaceOff
            | BackendItem::FakeSpace
            | BackendItem::RunningLinkOff
            | BackendItem::RunningLinkOn => {}
        }
        Ok(())
    }

    /// Places a form or image whose reference point is at (`h`, `v`).
    fn xobject(&mut self, item: &BackendItem, h: Scaled, v: Scaled) {
        let Some((w, ht, d)) = item.dimensions(&self.eng.stomach.backend) else {
            return;
        };
        let op = match item {
            BackendItem::RefXForm(n) => PdfOp::Form { obj: *n, depth: d },
            BackendItem::RefXImage(n) => PdfOp::Image {
                obj: *n,
                width: w,
                height: ht,
                depth: d,
            },
            _ => return,
        };
        self.pdf(h, v, op);
    }

    fn whatsit(&mut self, w: &Whatsit, h: Scaled, v: Scaled) -> Res {
        self.run_end = None;
        match w {
            Whatsit::Open { stream, name } => {
                if !self.doing_leaders {
                    let core = rtex_core::Whatsit::Open {
                        stream: *stream,
                        name: name.clone(),
                    };
                    self.eng.out_what(&core)?;
                }
            }
            Whatsit::Write { stream, toks } => {
                if !self.doing_leaders {
                    self.eng.write_out(*stream, toks.clone())?;
                }
            }
            Whatsit::Close { stream } => {
                if !self.doing_leaders {
                    self.eng.out_what(&rtex_core::Whatsit::Close { stream: *stream })?;
                }
            }
            Whatsit::Special(toks) => {
                let text = self.eng.token_list_string(toks);
                self.special(h, v, text);
            }
            Whatsit::Language { .. } => {}
            Whatsit::Backend(item) => self.backend(item, h, v)?,
        }
        Ok(())
    }

    fn hlist_out(&mut self, b: &BoxNode, left: Scaled, base_line: Scaled) -> Res {
        self.level += 1;
        self.parents.push(Parent {
            left,
            base: base_line,
            width: b.width,
            height: b.height,
            depth: b.depth,
        });
        let level = self.level;
        for l in self.links.iter_mut().filter(|l| l.level == level) {
            l.left = left;
            l.top = if l.rule.height == RUNNING {
                base_line - b.height
            } else {
                base_line - l.rule.height
            };
            l.bottom = if l.rule.depth == RUNNING {
                base_line + b.depth
            } else {
                base_line + l.rule.depth
            };
        }
        let r = self.hlist_body(b, left, base_line);
        let open: Vec<OpenLink> = self.links.iter().filter(|l| l.level == level).cloned().collect();
        for l in open {
            self.close_link(&l, left + b.width);
            if let Some(k) = self.links.iter_mut().find(|k| k.level == level && k.obj == l.obj) {
                k.obj = 0;
            }
        }
        self.parents.pop();
        self.level -= 1;
        r
    }

    fn hlist_body(&mut self, b: &BoxNode, left: Scaled, base_line: Scaled) -> Res {
        let mut cur_h = left;
        let mut cur_g: Scaled = 0;
        let mut cur_glue = 0.0f64;
        for p in &b.list {
            match p {
                Node::Whatsit(Whatsit::Backend(item)) if item.dimensions(&self.eng.stomach.backend).is_some() => {
                    self.xobject(item, cur_h, base_line);
                    cur_h += item.dimensions(&self.eng.stomach.backend).map_or(0, |d| d.0);
                }
                Node::Char { font, ch } => cur_h += self.glyph(*font, *ch, cur_h, base_line),
                Node::Ligature(l) => cur_h += self.glyph(l.font, l.ch, cur_h, base_line),
                Node::Box(inner) => {
                    if !inner.list.is_empty() {
                        let v = base_line + inner.shift;
                        match inner.dir {
                            BoxDir::H => self.hlist_out(inner, cur_h, v)?,
                            BoxDir::V => self.vlist_out(inner, cur_h, v - inner.height)?,
                        }
                        self.run_end = None;
                    }
                    cur_h += inner.width;
                }
                Node::Rule { width, depth, height } => {
                    let ht = if *height == RUNNING { b.height } else { *height };
                    let dp = if *depth == RUNNING { b.depth } else { *depth };
                    self.rule(cur_h, base_line - ht, *width, ht + dp);
                    cur_h += width;
                }
                Node::Whatsit(w) => self.whatsit(w, cur_h, base_line)?,
                Node::Glue(g) => {
                    let mut rule_wd = g.spec.width - cur_g;
                    match b.glue_sign {
                        GlueSign::Stretching if g.spec.stretch_order == b.glue_order => {
                            cur_glue += f64::from(g.spec.stretch);
                            cur_g = vet(b.glue_set * cur_glue).round() as Scaled;
                        }
                        GlueSign::Shrinking if g.spec.shrink_order == b.glue_order => {
                            cur_glue -= f64::from(g.spec.shrink);
                            cur_g = vet(b.glue_set * cur_glue).round() as Scaled;
                        }
                        _ => {}
                    }
                    rule_wd += cur_g;
                    if let Some(leader) = g.leader.as_deref().filter(|_| g.kind.is_leaders()) {
                        match leader {
                            Node::Rule { height, depth, .. } => {
                                let ht = if *height == RUNNING { b.height } else { *height };
                                let dp = if *depth == RUNNING { b.depth } else { *depth };
                                self.rule(cur_h, base_line - ht, rule_wd, ht + dp);
                                cur_h += rule_wd;
                                continue;
                            }
                            Node::Box(lb) => {
                                let leader_wd = lb.width;
                                if leader_wd > 0 && rule_wd > 0 {
                                    let rule_wd = rule_wd + 10;
                                    let edge = cur_h + rule_wd;
                                    let mut lx = 0;
                                    if g.kind == GlueKind::ALeaders {
                                        let save_h = cur_h;
                                        cur_h = left + leader_wd * (cur_h - left).div_euclid(leader_wd);
                                        if cur_h < save_h {
                                            cur_h += leader_wd;
                                        }
                                    } else {
                                        let lq = rule_wd / leader_wd;
                                        let lr = rule_wd % leader_wd;
                                        if g.kind == GlueKind::CLeaders {
                                            cur_h += lr / 2;
                                        } else {
                                            lx = lr / (lq + 1);
                                            cur_h += (lr - (lq - 1) * lx) / 2;
                                        }
                                    }
                                    while cur_h + leader_wd <= edge {
                                        let v = base_line + lb.shift;
                                        let outer = self.doing_leaders;
                                        self.doing_leaders = true;
                                        match lb.dir {
                                            BoxDir::H => self.hlist_out(lb, cur_h, v)?,
                                            BoxDir::V => self.vlist_out(lb, cur_h, v - lb.height)?,
                                        }
                                        self.doing_leaders = outer;
                                        self.run_end = None;
                                        cur_h += leader_wd + lx;
                                    }
                                    cur_h = edge - 10;
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }
                    cur_h += rule_wd;
                }
                Node::Kern { width, .. } | Node::Math { width, .. } | Node::MarginKern { width, .. } => cur_h += width,
                _ => {}
            }
        }
        Ok(())
    }

    fn vlist_out(&mut self, b: &BoxNode, left: Scaled, top: Scaled) -> Res {
        self.level += 1;
        self.parents.push(Parent {
            left,
            base: top + b.height,
            width: b.width,
            height: b.height,
            depth: b.depth,
        });
        let r = self.vlist_body(b, left, top);
        self.parents.pop();
        self.level -= 1;
        r
    }

    fn vlist_body(&mut self, b: &BoxNode, left: Scaled, top: Scaled) -> Res {
        let mut cur_v = top;
        let mut cur_g: Scaled = 0;
        let mut cur_glue = 0.0f64;
        for p in &b.list {
            match p {
                Node::Whatsit(Whatsit::Backend(item)) if item.dimensions(&self.eng.stomach.backend).is_some() => {
                    let (_, ht, dp) = item.dimensions(&self.eng.stomach.backend).unwrap_or_default();
                    cur_v += ht;
                    self.xobject(item, left, cur_v);
                    cur_v += dp;
                }
                Node::Box(inner) => {
                    if inner.list.is_empty() {
                        cur_v += inner.height + inner.depth;
                    } else {
                        cur_v += inner.height;
                        let h = left + inner.shift;
                        match inner.dir {
                            BoxDir::H => self.hlist_out(inner, h, cur_v)?,
                            BoxDir::V => self.vlist_out(inner, h, cur_v - inner.height)?,
                        }
                        self.run_end = None;
                        cur_v += inner.depth;
                    }
                }
                Node::Rule { width, depth, height } => {
                    let wd = if *width == RUNNING { b.width } else { *width };
                    let ht = height + depth;
                    self.rule(left, cur_v, wd, ht);
                    cur_v += ht;
                }
                Node::Whatsit(w) => self.whatsit(w, left, cur_v)?,
                Node::Glue(g) => {
                    let mut rule_ht = g.spec.width - cur_g;
                    match b.glue_sign {
                        GlueSign::Stretching if g.spec.stretch_order == b.glue_order => {
                            cur_glue += f64::from(g.spec.stretch);
                            cur_g = vet(b.glue_set * cur_glue).round() as Scaled;
                        }
                        GlueSign::Shrinking if g.spec.shrink_order == b.glue_order => {
                            cur_glue -= f64::from(g.spec.shrink);
                            cur_g = vet(b.glue_set * cur_glue).round() as Scaled;
                        }
                        _ => {}
                    }
                    rule_ht += cur_g;
                    if let Some(leader) = g.leader.as_deref().filter(|_| g.kind.is_leaders()) {
                        match leader {
                            Node::Rule { width, .. } => {
                                let wd = if *width == RUNNING { b.width } else { *width };
                                self.rule(left, cur_v, wd, rule_ht);
                                cur_v += rule_ht;
                                continue;
                            }
                            Node::Box(lb) => {
                                let leader_ht = lb.height + lb.depth;
                                if leader_ht > 0 && rule_ht > 0 {
                                    let rule_ht = rule_ht + 10;
                                    let edge = cur_v + rule_ht;
                                    let mut lx = 0;
                                    if g.kind == GlueKind::ALeaders {
                                        let save_v = cur_v;
                                        cur_v = top + leader_ht * (cur_v - top).div_euclid(leader_ht);
                                        if cur_v < save_v {
                                            cur_v += leader_ht;
                                        }
                                    } else {
                                        let lq = rule_ht / leader_ht;
                                        let lr = rule_ht % leader_ht;
                                        if g.kind == GlueKind::CLeaders {
                                            cur_v += lr / 2;
                                        } else {
                                            lx = lr / (lq + 1);
                                            cur_v += (lr - (lq - 1) * lx) / 2;
                                        }
                                    }
                                    while cur_v + leader_ht <= edge {
                                        let h = left + lb.shift;
                                        let outer = self.doing_leaders;
                                        self.doing_leaders = true;
                                        match lb.dir {
                                            BoxDir::H => self.hlist_out(lb, h, cur_v + lb.height)?,
                                            BoxDir::V => self.vlist_out(lb, h, cur_v)?,
                                        }
                                        self.doing_leaders = outer;
                                        self.run_end = None;
                                        cur_v += leader_ht + lx;
                                    }
                                    cur_v = edge - 10;
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }
                    cur_v += rule_ht;
                }
                Node::Kern { width, .. } => cur_v += width,
                _ => {}
            }
        }
        Ok(())
    }
}

/// `ship_out` (§638).
pub(crate) fn ship_out(eng: &mut Eng, b: BoxNode) -> Res {
    let tracing = eng.int_par(TRACING_OUTPUT) > 0;
    if tracing {
        eng.print_nl("");
        eng.print_ln();
        eng.print_str("Completed box being shipped out");
    }
    let max_print_line = eng.config().max_print_line;
    if eng.term_offset() > max_print_line.saturating_sub(9) {
        eng.print_ln();
    } else if eng.term_offset() > 0 || eng.file_offset() > 0 {
        eng.print_char(u32::from(' '));
    }
    eng.print_char(u32::from('['));
    let mut j = 9;
    while eng.eqtb().count(j) == 0 && j > 0 {
        j -= 1;
    }
    for k in 0..=j {
        eng.print_int(eng.eqtb().count(k));
        if k < j {
            eng.print_char(u32::from('.'));
        }
    }
    if tracing {
        eng.print_char(u32::from(']'));
        eng.begin_diagnostic();
        show_box_node(eng, &Node::Box(Box::new(b.clone())));
        eng.end_diagnostic(true);
    }
    let h_offset = eng.eqtb().dimen_par(HOFFSET);
    let v_offset = eng.eqtb().dimen_par(VOFFSET);
    if b.height > MAX_DIMEN
        || b.depth > MAX_DIMEN
        || b.height + b.depth + v_offset > MAX_DIMEN
        || b.width + h_offset > MAX_DIMEN
    {
        eng.print_err("Huge page cannot be shipped out");
        eng.set_help(&[
            "The page just created is more than 18 feet tall or",
            "more than 18 feet wide, so I suspect something went wrong.",
        ]);
        eng.error()?;
        if eng.int_par(super::TRACING_ONLINE) <= 0 {
            eng.begin_diagnostic();
            eng.print_nl("The following box has been deleted:");
            show_box_node(eng, &Node::Box(Box::new(b)));
            eng.end_diagnostic(true);
        }
    } else {
        let (width, height, left, top) = super::stomach::page_geometry(eng, &b);
        let page_number = eng.stomach.document.pages().len() + 1;
        eng.stomach.backend.page_object(page_number);
        let mut out = Out::new(eng, height, true);
        out.start_page_colors();
        match b.dir {
            BoxDir::H => out.hlist_out(&b, left, top + b.height)?,
            BoxDir::V => out.vlist_out(&b, left, top)?,
        }
        let ops = out.ops;
        let attr = page_toks(eng, super::pdftex::toks::PAGE_ATTR);
        let resources = page_toks(eng, super::pdftex::toks::PAGE_RESOURCES);
        eng.stomach.document.push_page(Page {
            width,
            height,
            ops,
            attr,
            resources,
        });
    }
    if !tracing {
        eng.print_char(u32::from(']'));
    }
    eng.aux.dead_cycles = 0;
    Ok(())
}

impl Out<'_> {
    /// Re-emits the current colour of every page-start colour stack (`pdf_out_colorstack_startpage`).
    fn start_page_colors(&mut self) {
        let stacks: Vec<(LiteralMode, String)> = self
            .eng
            .stomach
            .backend
            .color_stacks
            .iter()
            .filter(|s| s.page_start)
            .filter_map(|s| s.values.last().map(|v| (s.mode, v.clone())))
            .filter(|(_, v)| v != "0 g 0 G" && !v.is_empty())
            .collect();
        for (mode, v) in stacks {
            self.literal(0, 0, mode, v);
        }
    }
}

/// An action with its token lists turned into text.
pub(crate) fn resolve_action(eng: &mut Eng, a: &Action) -> PdfAction {
    let target = match &a.target {
        ActionTarget::User(t) => PdfTarget::User(eng.token_list_string(t)),
        ActionTarget::GoTo(Ident::Name(t)) => PdfTarget::Name(eng.token_list_string(t)),
        ActionTarget::GoTo(Ident::Num(n)) => PdfTarget::Num(*n),
        ActionTarget::Page(n, t) => PdfTarget::Page(*n, eng.token_list_string(t)),
        ActionTarget::Thread(Ident::Name(t)) => PdfTarget::ThreadName(eng.token_list_string(t)),
        ActionTarget::Thread(Ident::Num(n)) => PdfTarget::ThreadNum(*n),
    };
    let file = a.file.as_ref().map(|f| eng.token_list_string(f));
    PdfAction {
        target,
        file,
        new_window: a.new_window,
    }
}

fn page_toks(eng: &mut Eng, k: usize) -> String {
    match super::pdftex::pdf_toks(eng, k) {
        Some(t) => eng.token_list_string(&t),
        None => String::new(),
    }
}

/// The page operations of a `\pdfxform` box, with the origin at its lower-left corner.
pub(crate) fn render_form(eng: &mut Eng, b: &BoxNode) -> Res<Vec<DrawOp>> {
    let height = b.height + b.depth;
    let mut out = Out::new(eng, height, false);
    match b.dir {
        BoxDir::H => out.hlist_out(b, 0, b.height)?,
        BoxDir::V => out.vlist_out(b, 0, 0)?,
    }
    Ok(out.ops)
}

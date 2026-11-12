//! The [`Stomach`] implementation: TeX's `main_control` cases that build lists (§1045).

use std::sync::Arc;

use rtex_core::cmd::{self, chr};
use rtex_core::{
    BoxContext as CoreBoxContext, BoxGroupContext, BoxKind, BoxSpec, Engine, FontInfo, GlueRef, GroupKind, Internal,
    Res, Stomach, Token,
};

use super::build;
use super::chars::{append_normal_space, append_space, main_loop};
use super::node::{BoxDir, GlueKind, KernKind, Node, Whatsit};
use super::{
    BoxContext, Eng, GroupData, HMODE, MMODE, Spec, Typesetter, VMODE, abs_mode, align, fonts, hyphen, math, page,
    show, tail_append,
};
use crate::units::Scaled;

impl Stomach for Typesetter {
    fn command(eng: &mut Eng) -> Res {
        let c = eng.cur_cmd();
        let m = abs_mode(eng);
        match c {
            cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN if m == HMODE => main_loop(eng, eng.cur_chr()),
            cmd::CHAR_NUM if m == HMODE => {
                let ch = eng.scan_char_num()? as u32;
                main_loop(eng, ch)
            }
            cmd::NO_BOUNDARY if m == HMODE => {
                eng.get_x_token()?;
                if matches!(
                    eng.cur_cmd(),
                    cmd::LETTER | cmd::OTHER_CHAR | cmd::CHAR_GIVEN | cmd::CHAR_NUM
                ) {
                    eng.stomach.cancel_boundary = true;
                }
                eng.reswitch();
                Ok(())
            }
            cmd::SPACER if m == HMODE => {
                append_space(eng);
                Ok(())
            }
            cmd::EX_SPACE if m == HMODE || m == MMODE => {
                append_normal_space(eng);
                Ok(())
            }
            cmd::LETTER
            | cmd::OTHER_CHAR
            | cmd::CHAR_NUM
            | cmd::CHAR_GIVEN
            | cmd::MATH_SHIFT
            | cmd::UN_HBOX
            | cmd::VRULE
            | cmd::ACCENT
            | cmd::DISCRETIONARY
            | cmd::HSKIP
            | cmd::VALIGN
            | cmd::EX_SPACE
            | cmd::NO_BOUNDARY
                if m == VMODE =>
            {
                eng.back_input()?;
                build::new_graf(eng, true)
            }
            cmd::START_PAR if m == VMODE => build::new_graf(eng, eng.cur_chr() > 0),
            cmd::START_PAR => {
                if eng.cur_chr() != 2 {
                    build::indent_in_hmode(eng)?;
                }
                Ok(())
            }
            cmd::VSKIP if m == VMODE => build::append_glue(eng),
            cmd::HSKIP if m != VMODE => build::append_glue(eng),
            cmd::MSKIP if m == MMODE => build::append_glue(eng),
            cmd::KERN => build::append_kern(eng),
            cmd::MKERN if m == MMODE => build::append_kern(eng),
            cmd::HMOVE if m == VMODE => shifted_box(eng),
            cmd::VMOVE if m != VMODE => shifted_box(eng),
            cmd::LEADER_SHIP => {
                let ctx = match eng.cur_chr() {
                    99 => BoxContext::ShipOut,
                    101 => BoxContext::Leaders(GlueKind::CLeaders),
                    102 => BoxContext::Leaders(GlueKind::XLeaders),
                    _ => BoxContext::Leaders(GlueKind::ALeaders),
                };
                build::scan_box(eng, ctx)
            }
            cmd::MAKE_BOX => build::begin_box(eng, BoxContext::Shift(0)),
            cmd::PAR_END if m == VMODE => {
                build::normal_paragraph(eng)?;
                if eng.mode > 0 {
                    page::build_page(eng)?;
                }
                Ok(())
            }
            cmd::PAR_END if m == HMODE => {
                if *eng.align_state_mut() < 0 {
                    eng.off_save()?;
                }
                build::end_graf(eng)?;
                if eng.mode == VMODE {
                    page::build_page(eng)?;
                }
                Ok(())
            }
            cmd::STOP | cmd::VSKIP | cmd::HRULE | cmd::UN_VBOX | cmd::HALIGN if m == HMODE => {
                build::head_for_vmode(eng)
            }
            cmd::HRULE if m == VMODE => build::append_rule(eng),
            cmd::VRULE => build::append_rule(eng),
            cmd::INSERT => build::begin_insert_or_adjust(eng),
            cmd::VADJUST if m != VMODE => build::begin_insert_or_adjust(eng),
            cmd::MARK => build::make_mark(eng),
            cmd::BREAK_PENALTY => build::append_penalty(eng),
            cmd::REMOVE_ITEM => build::delete_last(eng),
            cmd::UN_VBOX if m == VMODE => build::unpackage(eng),
            cmd::UN_HBOX => build::unpackage(eng),
            cmd::ITAL_CORR if m == HMODE => build::append_italic_correction(eng),
            cmd::ITAL_CORR if m == MMODE => {
                tail_append(eng, Node::kern(0, KernKind::Normal));
                Ok(())
            }
            cmd::DISCRETIONARY => build::append_discretionary(eng),
            cmd::ACCENT if m == HMODE => build::make_accent(eng),
            cmd::CAR_RET | cmd::TAB_MARK => build::align_error(eng),
            cmd::NO_ALIGN => build::misplaced(eng, "noalign"),
            cmd::OMIT => build::misplaced(eng, "omit"),
            cmd::HALIGN if m == VMODE => align::init_align(eng),
            cmd::VALIGN if m == HMODE => align::init_align(eng),
            cmd::HALIGN if m == MMODE => math::halign_in_display(eng),
            cmd::ENDV if m == VMODE || m == HMODE => align::do_endv(eng),
            cmd::MATH_SHIFT if m == HMODE => math::init_math(eng),
            cmd::SPACER | cmd::NO_BOUNDARY => Ok(()),
            cmd::VMOVE | cmd::HMOVE | cmd::LAST_ITEM | cmd::MAC_PARAM => eng.report_illegal_case(),
            cmd::VADJUST | cmd::ITAL_CORR => eng.report_illegal_case(),
            cmd::EQ_NO if m != MMODE => eng.report_illegal_case(),
            cmd::SUP_MARK
            | cmd::SUB_MARK
            | cmd::MATH_CHAR_NUM
            | cmd::MATH_GIVEN
            | cmd::MATH_COMP
            | cmd::DELIM_NUM
            | cmd::LEFT_RIGHT
            | cmd::ABOVE
            | cmd::RADICAL
            | cmd::MATH_STYLE
            | cmd::MATH_CHOICE
            | cmd::VCENTER
            | cmd::NON_SCRIPT
            | cmd::MKERN
            | cmd::LIMIT_SWITCH
            | cmd::MSKIP
            | cmd::MATH_ACCENT
                if m != MMODE =>
            {
                build::insert_dollar_sign(eng)
            }
            cmd::ENDV | cmd::PAR_END | cmd::STOP | cmd::VSKIP | cmd::UN_VBOX | cmd::VALIGN | cmd::HRULE
                if m == MMODE =>
            {
                build::insert_dollar_sign(eng)
            }
            cmd::EXTENSION => super::pdftex::command(eng),
            _ if m == MMODE => math::command(eng),
            _ => eng.report_illegal_case(),
        }
    }

    fn internal(eng: &mut Eng, c: u8, m: u32) -> Res<Option<Internal>> {
        Ok(match c {
            cmd::LAST_ITEM if m <= chr::LAST_NODE_TYPE => Some(last_item(eng, m)),
            cmd::LAST_ITEM if m == chr::BADNESS => Some(Internal::Int(eng.stomach.last_badness)),
            cmd::LAST_ITEM if m >= chr::STOMACH_BASE => super::pdftex::internal(eng, m)?,
            cmd::ASSIGN_FONT_INT if m >= chr::STOMACH_BASE => super::pdftex::font_code_internal(eng, m)?,
            cmd::SET_BOX_DIMEN => {
                let n = eng.scan_register_num()? as u16;
                let v = build::fetch_box(eng, n).map_or(0, |b| match m {
                    1 => b.width,
                    3 => b.height,
                    _ => b.depth,
                });
                Some(Internal::Dimen(v))
            }
            cmd::SET_PREV_GRAF => Some(Internal::Int(if eng.mode == 0 { 0 } else { prev_graf_of_vlist(eng) })),
            cmd::SET_PAGE_DIMEN => {
                let p = &eng.stomach.page;
                let v = if p.contents == page::PageContents::Empty && !p.output_active {
                    if m == 0 { super::node::MAX_DIMEN } else { 0 }
                } else {
                    eng.aux.page_so_far[m as usize]
                };
                Some(Internal::Dimen(v))
            }
            _ => None,
        })
    }

    fn assign(eng: &mut Eng, c: u8, m: u32, global: bool) -> Res {
        match c {
            cmd::SET_BOX => build::set_box(eng, global),
            cmd::SET_BOX_DIMEN => build::alter_box_dimen(eng),
            cmd::ASSIGN_FONT_INT if m >= chr::STOMACH_BASE => super::pdftex::assign_font_code(eng, m),
            cmd::SET_PREV_GRAF => alter_prev_graf(eng),
            cmd::HYPH_DATA if m == 1 => hyphen::new_patterns(eng),
            cmd::HYPH_DATA => hyphen::new_hyph_exceptions(eng),
            _ => eng.default_assign(c, m, global),
        }
    }

    fn its_all_over(eng: &mut Eng) -> Res<bool> {
        build::its_all_over(eng)
    }

    fn align_entry_end(eng: &mut Eng) -> Res {
        align::insert_v_part(eng)
    }

    fn runaway_preamble(eng: &Engine<Self>) -> Vec<Token> {
        eng.stomach.preamble_tokens.clone()
    }

    fn end_group(eng: &mut Eng, kind: GroupKind) -> Res {
        match kind {
            GroupKind::HBox => build::package(eng),
            GroupKind::AdjustedHBox => {
                eng.stomach.adjust = Some(Vec::new());
                build::package(eng)
            }
            GroupKind::VBox | GroupKind::VTop => {
                build::end_graf(eng)?;
                build::package(eng)
            }
            GroupKind::Insert => build::finish_insert(eng),
            GroupKind::Output => page::resume_after_output(eng),
            GroupKind::Disc => build::build_discretionary(eng),
            GroupKind::Align => align::end_align_group(eng),
            GroupKind::NoAlign => align::end_no_align(eng),
            GroupKind::VCenter | GroupKind::MathChoice | GroupKind::Math => math::end_group(eng, kind),
            _ => eng.unsave(),
        }
    }

    fn box_kind(eng: &Eng, handle: i32) -> BoxKind {
        match eng.stomach.boxes.get(&handle) {
            Some(b) if b.dir == BoxDir::H => BoxKind::HBox,
            Some(_) => BoxKind::VBox,
            None => BoxKind::Void,
        }
    }

    fn load_font(eng: &mut Eng, file: &[u32], size: i32) -> Res<Option<FontInfo>> {
        fonts::load_font(eng, file, size)
    }

    fn letterspace_font(eng: &mut Eng, source: usize, e: i32) -> Res<Option<FontInfo>> {
        fonts::letterspace_font(eng, source, e)
    }

    fn copy_font(eng: &mut Eng, source: usize) -> Res<Option<FontInfo>> {
        fonts::copy_font(eng, source)
    }

    fn font_has_char(eng: &Eng, f: i32, c: u32) -> bool {
        usize::try_from(f).is_ok_and(|f| eng.stomach.fonts.has_char(f, c))
    }

    fn release_box(eng: &mut Eng, handle: i32) {
        eng.stomach.boxes.remove(&handle);
    }

    fn whatsit(eng: &mut Eng, w: rtex_core::Whatsit) -> Res {
        let node = match w {
            rtex_core::Whatsit::Open { stream, name } => Whatsit::Open { stream, name },
            rtex_core::Whatsit::Write { stream, toks } => Whatsit::Write { stream, toks },
            rtex_core::Whatsit::Close { stream } => Whatsit::Close { stream },
            rtex_core::Whatsit::Special(toks) => Whatsit::Special(toks),
            rtex_core::Whatsit::Language(n) => {
                if abs_mode(eng) != HMODE {
                    return eng.report_illegal_case();
                }
                let lang = if (1..=255).contains(&n) { n } else { 0 };
                eng.stomach.clang = lang;
                Whatsit::Language {
                    lang,
                    lhm: eng.int_par(51).clamp(1, 63),
                    rhm: eng.int_par(52).clamp(1, 63),
                }
            }
        };
        tail_append(eng, Node::Whatsit(node));
        Ok(())
    }

    fn mark(eng: &Eng, code: u32, class: i32) -> Option<Arc<[Token]>> {
        page::mark_text(eng, code, class)
    }

    fn show(eng: &mut Eng, c: u32) -> Res {
        if c == chr::SHOW_BOX {
            build::show_box_register(eng)
        } else {
            eng.begin_diagnostic();
            show::show_activities(eng);
            Ok(())
        }
    }

    fn expand(eng: &mut Eng, c: u32) -> Res {
        super::pdftex::expand(eng, c)
    }

    fn box_group_context(eng: &Engine<Self>, level: usize) -> Option<BoxGroupContext> {
        eng.stomach.groups.iter().find_map(|g| match g {
            GroupData::Box {
                context,
                spec,
                level: l,
                ..
            } if usize::from(*l) == level => Some(BoxGroupContext {
                context: match context {
                    BoxContext::Shift(0) => CoreBoxContext::Plain,
                    BoxContext::Shift(d) => CoreBoxContext::Shifted {
                        cmd: if abs_mode(eng) == VMODE {
                            if *d > 0 { "moveright" } else { "moveleft" }
                        } else if *d > 0 {
                            "lower"
                        } else {
                            "raise"
                        },
                        amount: *d,
                    },
                    BoxContext::SetBox { register, global } => CoreBoxContext::SetBox {
                        register: i32::from(*register),
                        global: *global,
                    },
                    BoxContext::ShipOut => CoreBoxContext::Command("shipout"),
                    BoxContext::Leaders(GlueKind::CLeaders) => CoreBoxContext::Command("cleaders"),
                    BoxContext::Leaders(GlueKind::XLeaders) => CoreBoxContext::Command("xleaders"),
                    BoxContext::Leaders(_) => CoreBoxContext::Command("leaders"),
                },
                spec: match spec {
                    Spec::Exactly(d) => BoxSpec::To(*d),
                    Spec::Additional(0) => BoxSpec::Natural,
                    Spec::Additional(d) => BoxSpec::Spread(*d),
                },
            }),
            _ => None,
        })
    }

    fn dump(eng: &Engine<Self>) -> Vec<u8> {
        super::format::dump(eng)
    }

    fn undump(eng: &mut Engine<Self>, data: &[u8]) -> bool {
        super::format::undump(eng, data)
    }

    fn finish(eng: &mut Eng) {
        let pages = eng.stomach.document.pages().len();
        if eng.history() == rtex_core::History::FatalErrorStop
            && super::pdftex::pdf_int(eng, super::pdftex::int::OUTPUT) > 0
        {
            eng.print_err(" ==> Fatal error occurred, no output PDF file produced!");
            return;
        }
        if pages == 0 {
            eng.print_nl("No pages of output.");
            return;
        }
        let Ok(job) = super::pdftex::pdf_job(eng) else { return };
        let tfms: Vec<String> = eng.stomach.document.fonts().map(|(_, f)| f.name.clone()).collect();
        let mut library =
            crate::fonts::library::FontLibrary::new(eng.stomach.font_map.clone(), eng.stomach.encodings.clone());
        let shown = library.load_programs(tfms.iter().map(String::as_str), eng.file_system_mut());
        for name in shown {
            eng.print_char(u32::from('<'));
            eng.print_str(&name);
            eng.print_char(u32::from('>'));
        }
        let out = crate::pdf::write_pdf(&eng.stomach.document, &job, &library);
        for w in &out.warnings {
            eng.print_nl("");
            eng.print_str(w);
            eng.print_ln();
            eng.warning_issued();
        }
        let pdf = out.bytes;
        eng.print_nl("Output written on ");
        let name = eng.stomach.output_name.clone();
        eng.print_str(&name);
        eng.print_str(" (");
        eng.print_int(pages as i32);
        eng.print_str(if pages == 1 { " page" } else { " pages" });
        eng.print_str(", ");
        eng.print_int(pdf.len() as i32);
        eng.print_str(" bytes).");
        eng.stomach.pdf = Some(pdf);
    }
}

fn shifted_box(eng: &mut Eng) -> Res {
    let t = eng.cur_chr();
    let d = eng.scan_normal_dimen()?;
    build::scan_box(eng, BoxContext::Shift(if t == 0 { d } else { -d }))
}

fn last_item(eng: &Eng, m: u32) -> Internal {
    let tail = if eng.mode == 0 { None } else { eng.stomach.cur.last() };
    let empty = eng.stomach.cur.is_empty() || eng.mode == 0;
    let at_page = eng.mode == VMODE && eng.stomach.cur.is_empty();
    match m {
        chr::LAST_PENALTY => Internal::Int(match tail {
            Some(Node::Penalty(p)) => *p,
            None if at_page => eng.stomach.page.last_penalty,
            _ => 0,
        }),
        chr::LAST_KERN => Internal::Dimen(match tail {
            Some(Node::Kern { width, .. }) => *width,
            None if at_page => eng.stomach.page.last_kern,
            _ => 0,
        }),
        chr::LAST_SKIP => match tail {
            Some(Node::Glue(g)) => {
                let r = GlueRef(Arc::new(g.spec));
                if g.kind == GlueKind::Mu {
                    Internal::Mu(r)
                } else {
                    Internal::Glue(r)
                }
            }
            None if at_page => Internal::Glue(
                eng.stomach
                    .page
                    .last_glue
                    .map_or_else(|| eng.eqtb().zero_glue().clone(), |g| GlueRef(Arc::new(g))),
            ),
            _ => Internal::Glue(eng.eqtb().zero_glue().clone()),
        },
        _ => Internal::Int(match tail {
            _ if at_page => eng.stomach.page.last_node_type,
            _ if empty => -1,
            Some(n) => n.type_code(),
            None => -1,
        }),
    }
}

fn prev_graf_of_vlist(eng: &Eng) -> i32 {
    if eng.mode.abs() == VMODE {
        return eng.stomach.prev_graf;
    }
    for s in eng.stomach.nest.iter().rev() {
        if s.mode.abs() == VMODE {
            return s.prev_graf;
        }
    }
    0
}

/// `alter_prev_graf` (§1244).
fn alter_prev_graf(eng: &mut Eng) -> Res {
    eng.scan_optional_equals()?;
    let v = eng.scan_int()?;
    if v < 0 {
        eng.print_err("Bad ");
        eng.print_esc("prevgraf");
        eng.set_help(&["I allow only nonnegative values here."]);
        return eng.int_error(v);
    }
    if eng.mode.abs() == VMODE {
        eng.stomach.prev_graf = v;
    } else if let Some(s) = eng.stomach.nest.iter_mut().rev().find(|s| s.mode.abs() == VMODE) {
        s.prev_graf = v;
    }
    Ok(())
}

/// The media size of a page holding `b` and where `b`'s top-left corner goes (pdfTeX's `pdf_ship_out`).
pub(crate) fn page_geometry(eng: &Eng, b: &super::node::BoxNode) -> (Scaled, Scaled, Scaled, Scaled) {
    use super::pdftex::{dimen, pdf_dimen};
    let left = pdf_dimen(eng, dimen::H_ORIGIN) + eng.eqtb().dimen_par(HOFFSET);
    let top = pdf_dimen(eng, dimen::V_ORIGIN) + eng.eqtb().dimen_par(VOFFSET);
    let w = match pdf_dimen(eng, dimen::PAGE_WIDTH) {
        0 => b.width + 2 * left,
        w => w,
    };
    let h = match pdf_dimen(eng, dimen::PAGE_HEIGHT) {
        0 => b.height + b.depth + 2 * top,
        h => h,
    };
    (w, h, left, top)
}

const HOFFSET: usize = 18;
const VOFFSET: usize = 19;

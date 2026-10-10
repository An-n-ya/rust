//! MIR pass implementing an FFI-boundary data race sanitizer.
//!
//! Instead of instrumenting every memory access (which is what a full
//! ThreadSanitizer build does), this pass only reports accesses that cross the
//! Rust/C (FFI) boundary. Concretely it inserts calls to the `ffi_race_*`
//! runtime wrappers defined in `core::ffi_race_sanitizer`:
//!
//! * R1: raw-pointer arguments of an FFI call are reported as a read
//!   (`*const T`) or a write (`*mut T`) right before the call.
//! * R2: dereferences of raw pointers inside FFI-related functions are
//!   reported as reads/writes according to their `PlaceContext`.
//! * R3: raw pointers captured by a `thread::spawn` closure get a
//!   `release` before the spawn and an `acquire` at the start of the closure.
//! * R4: calls to known lock-protected C APIs are bracketed with
//!   `mutex_pre_lock`/`mutex_post_lock` and the matching unlock annotations.
//!
//! The wrappers are located through `#[rustc_diagnostic_item]` names, so this
//! pass only has an effect when the `ffi_race_sanitizer` feature is enabled.
//!
//! Several small helpers (`is_ffi_call`, `has_ffi_call`, `can_be_ffi_call`,
//! `is_main_function`, `get_temp_local`, `unwind_for_block`) are intentionally
//! duplicated from `ffi_sanitizer.rs` so that this feature does not depend on
//! the (separate) pointer-bug sanitizer.

use rustc_abi::{ExternAbi, FieldIdx};
use rustc_hir::Attribute;
use rustc_hir::attrs::AttributeKind;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::*;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::DUMMY_SP;
use rustc_span::source_map::dummy_spanned;
use rustc_span::symbol::{Symbol, sym};

/// C APIs that are known to take a lock internally. Calls to these functions
/// are bracketed with TSan mutex annotations so that the accesses reported at
/// the boundary are treated as ordered rather than as a data race.
const KNOWN_LOCKED_FFI_APIS: &[&str] = &["c_locked_touch"];

pub(super) struct FFIRaceSanitizer;

impl<'tcx> crate::MirPass<'tcx> for FFIRaceSanitizer {
    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
        if !tcx.features().ffi_race_sanitizer() {
            return;
        }

        // R3 (acquire side): a closure that captures a raw pointer establishes
        // the acquire half of a happens-before relation for it.
        if is_closure_body(tcx, body) && has_raw_ptr_upvar(tcx, body) {
            instrument_closure_acquire(tcx, body);
        }

        let ffi_related =
            has_ffi_call(tcx, body) || can_be_ffi_call(tcx, body) || is_main_function(tcx, body);
        if ffi_related {
            // R1 + R2 + R4.
            instrument_ffi_boundary(tcx, body);
        }

        // R3 (release side): hand raw pointers off to a spawned thread.
        if ffi_related && has_spawn_call(tcx, body) {
            instrument_spawn_handoff(tcx, body);
        }
    }

    fn is_required(&self) -> bool {
        true
    }
}

/// A single instrumentation call to insert: `wrapper(cast(arg))`.
#[derive(Clone)]
struct Instr<'tcx> {
    /// One of the `ffi_race_*` diagnostic item names.
    wrapper: &'static str,
    /// The pointee type the wrapper is generic over.
    pointee: Ty<'tcx>,
    /// The raw pointer operand to pass to the wrapper.
    arg: Operand<'tcx>,
    /// Whether the wrapper expects `*mut T` (write side) instead of `*const T`.
    want_mut: bool,
}

fn is_ffi_call<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>, func: &Operand<'tcx>) -> bool {
    if let ty::FnDef(def_id, _) = func.ty(body, tcx).kind() {
        return tcx.is_foreign_item(*def_id);
    }
    false
}

fn has_ffi_call<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> bool {
    for (_, block) in body.basic_blocks.iter_enumerated() {
        let Some(terminator) = &block.terminator else { continue };
        if let TerminatorKind::Call { func, .. } = &terminator.kind {
            if is_ffi_call(tcx, body, func) {
                return true;
            }
        }
    }
    false
}

fn can_be_ffi_call<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> bool {
    let def_id = body.source.def_id().expect_local();

    let mut no_mangle = tcx.has_attr(def_id, sym::no_mangle);
    for attr in tcx.get_all_attrs(def_id).iter() {
        if let Attribute::Parsed(parsed) = attr {
            no_mangle |= match parsed {
                AttributeKind::NoMangle(_) => true,
                AttributeKind::ExportName { .. } => true,
                _ => false,
            };
        }
    }

    if no_mangle
        && tcx.visibility(def_id).is_public()
        && let ExternAbi::C { .. } = tcx.fn_sig(def_id).skip_binder().abi()
    {
        return true;
    }
    false
}

fn is_main_function<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> bool {
    let def_id = body.source.def_id().expect_local();
    if let Some((entry_def_id, _)) = tcx.entry_fn(()) {
        entry_def_id == def_id.to_def_id()
    } else {
        false
    }
}

fn is_closure_body<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> bool {
    matches!(tcx.def_kind(body.source.def_id()), DefKind::Closure)
}

/// Collect the raw-pointer upvars captured by a closure body.
///
/// A closure body's first argument is `&{closure}` (a reference to the closure
/// environment), so the captured raw pointers are reached through a `deref`
/// projection followed by the field index of the upvar, e.g. `((*_1).0)`.
fn raw_ptr_upvar_places<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
) -> Vec<(Place<'tcx>, Ty<'tcx>)> {
    let mut result = Vec::new();
    let env_local = Local::from_usize(1);
    if body.local_decls.len() <= env_local.as_usize() {
        return result;
    }
    let env_ty = body.local_decls[env_local].ty;
    let (projection, adt_ty) = match env_ty.kind() {
        ty::Ref(_, inner, _) => (vec![PlaceElem::Deref], *inner),
        _ => (Vec::new(), env_ty),
    };
    let ty::Closure(_, args) = adt_ty.kind() else { return result };
    for (idx, upvar_ty) in args.as_closure().upvar_tys().iter().enumerate() {
        if let ty::RawPtr(pointee, _) = upvar_ty.kind() {
            let mut proj = projection.clone();
            proj.push(PlaceElem::Field(FieldIdx::from_usize(idx), upvar_ty));
            let projection = tcx.mk_place_elems(&proj);
            result.push((Place { local: env_local, projection }, *pointee));
        }
    }
    result
}

fn has_raw_ptr_upvar<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> bool {
    !raw_ptr_upvar_places(tcx, body).is_empty()
}

fn has_spawn_call<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> bool {
    for (_, block) in body.basic_blocks.iter_enumerated() {
        let Some(terminator) = &block.terminator else { continue };
        if let TerminatorKind::Call { func, .. } = &terminator.kind {
            if is_spawn_call(tcx, func) {
                return true;
            }
        }
    }
    false
}

fn is_spawn_call<'tcx>(tcx: TyCtxt<'tcx>, func: &Operand<'tcx>) -> bool {
    func.const_fn_def().is_some_and(|(def_id, _)| is_spawn_def(tcx, def_id))
}

fn is_spawn_def(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    // `def_path_str` can trigger the (expensive) `trimmed_def_paths` query, so
    // wrap it to keep this pure control-flow check from looking like a
    // diagnostic-producing path to the compiler's sanity checks.
    let path = rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(def_id));
    path == "std::thread::spawn"
        || path == "std::thread::Builder::spawn"
        || path.ends_with("::thread::spawn")
        || path.ends_with("::thread::Builder::spawn")
}

fn is_known_locked_api(tcx: TyCtxt<'_>, def_id: DefId) -> bool {
    let name = tcx.item_name(def_id);
    KNOWN_LOCKED_FFI_APIS.iter().any(|&api| name.as_str() == api)
}

fn get_temp_local<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) -> Place<'tcx> {
    let local_decl = LocalDecl::new(tcx.types.unit, DUMMY_SP);
    let local = body.local_decls.push(local_decl);
    Place::from(local)
}

fn new_temp<'tcx>(body: &mut Body<'tcx>, ty: Ty<'tcx>) -> Place<'tcx> {
    let local = body.local_decls.push(LocalDecl::new(ty, DUMMY_SP));
    Place::from(local)
}

/// Returns the unwind action newly-inserted call terminators must use. In a
/// function that cannot unwind (e.g. `extern "C"` with abort semantics) the MIR
/// validator rejects `UnwindAction::Continue`, so we fall back to
/// `UnwindAction::Unreachable` there.
fn unwind_for_block<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> UnwindAction {
    let def_id = body.source.def_id();
    let can_unwind = if !tcx.def_kind(def_id).is_fn_like() {
        true
    } else {
        let body_ty = tcx.type_of(def_id).skip_binder();
        let body_abi = match body_ty.kind() {
            ty::FnDef(..) => body_ty.fn_sig(tcx).abi(),
            ty::Closure(..) => ExternAbi::RustCall,
            ty::CoroutineClosure(..) => ExternAbi::RustCall,
            ty::Coroutine(..) => ExternAbi::Rust,
            _ => ExternAbi::Rust,
        };
        rustc_middle::ty::layout::fn_can_unwind(tcx, Some(def_id), body_abi)
    };
    if can_unwind { UnwindAction::Continue } else { UnwindAction::Unreachable }
}

/// Materialise `instr.arg` as an operand of exactly the pointer type the
/// wrapper expects, inserting a `PtrToPtr` cast into `current` when needed.
fn materialize_arg<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mut Body<'tcx>,
    current: BasicBlock,
    source_info: SourceInfo,
    instr: &Instr<'tcx>,
) -> Operand<'tcx> {
    let mutbl = if instr.want_mut { ty::Mutability::Mut } else { ty::Mutability::Not };
    let wanted = Ty::new_ptr(tcx, instr.pointee, mutbl);
    let arg_ty = instr.arg.ty(&body.local_decls, tcx);
    if arg_ty == wanted {
        return instr.arg.clone();
    }
    let tmp = new_temp(body, wanted);
    let rvalue = Rvalue::Cast(CastKind::PtrToPtr, instr.arg.clone(), wanted);
    body[current]
        .statements
        .push(Statement::new(source_info, StatementKind::Assign(Box::new((tmp, rvalue)))));
    Operand::Move(tmp)
}

/// Insert `instrs` (as calls) before the terminator of `current`, followed by
/// `tail_stmts` and the original terminator. Returns the block that now holds
/// the tail statements and the original terminator.
fn insert_before_term<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mut Body<'tcx>,
    current: BasicBlock,
    instrs: &[Instr<'tcx>],
    tail_stmts: Vec<Statement<'tcx>>,
) -> BasicBlock {
    let source_info = body[current].terminator().source_info;
    let unwind = unwind_for_block(tcx, body);

    let mut resolved: Vec<(DefId, Operand<'tcx>, Ty<'tcx>)> = Vec::new();
    for instr in instrs {
        let Some(def_id) = tcx.get_diagnostic_item(Symbol::intern(instr.wrapper)) else {
            // Wrapper not available: keep the tail statements, do not lose them.
            body[current].statements.extend(tail_stmts);
            return current;
        };
        let arg = materialize_arg(tcx, body, current, source_info, instr);
        resolved.push((def_id, arg, instr.pointee));
    }
    if resolved.is_empty() {
        body[current].statements.extend(tail_stmts);
        return current;
    }

    let orig_term = body[current].terminator().clone();

    let mut call_bbs = Vec::with_capacity(resolved.len());
    for (def_id, arg, pointee) in resolved {
        let func = Operand::function_handle(tcx, def_id, [pointee.into()], source_info.span);
        let terminator = Terminator {
            source_info,
            kind: TerminatorKind::Call {
                func,
                args: [dummy_spanned(arg)].into(),
                destination: get_temp_local(tcx, body),
                target: Some(BasicBlock::ZERO),
                unwind,
                call_source: CallSource::Misc,
                fn_span: DUMMY_SP,
            },
        };
        call_bbs.push(body.basic_blocks_mut().push(BasicBlockData::new(Some(terminator), false)));
    }

    let last_bb =
        body.basic_blocks_mut().push(BasicBlockData::new_stmts(tail_stmts, Some(orig_term), false));

    *body[current].terminator_mut() = Terminator {
        source_info,
        kind: TerminatorKind::Goto { target: call_bbs[0] },
    };

    for (idx, &bb) in call_bbs.iter().enumerate() {
        let target = call_bbs.get(idx + 1).copied().unwrap_or(last_bb);
        body[bb].terminator_mut().successors_mut(|succ| *succ = target);
    }

    last_bb
}

/// Insert `instrs` (as calls) after the FFI call terminator of `call_block`,
/// before its original target block.
fn insert_after_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mut Body<'tcx>,
    call_block: BasicBlock,
    instrs: &[Instr<'tcx>],
) {
    if instrs.is_empty() {
        return;
    }
    let (final_target, unwind) = match &body[call_block].terminator().kind {
        TerminatorKind::Call { target: Some(target), unwind, .. } => (*target, *unwind),
        _ => return,
    };
    let source_info = body[call_block].terminator().source_info;

    let mut resolved: Vec<(DefId, Operand<'tcx>, Ty<'tcx>)> = Vec::new();
    for instr in instrs {
        let Some(def_id) = tcx.get_diagnostic_item(Symbol::intern(instr.wrapper)) else { return };
        // Statements in `call_block` run before the FFI call, so casting the
        // mutex pointer here is safe and does not disturb the call itself.
        let arg = materialize_arg(tcx, body, call_block, source_info, instr);
        resolved.push((def_id, arg, instr.pointee));
    }

    let mut call_bbs = Vec::with_capacity(resolved.len());
    for (def_id, arg, pointee) in resolved {
        let func = Operand::function_handle(tcx, def_id, [pointee.into()], source_info.span);
        let terminator = Terminator {
            source_info,
            kind: TerminatorKind::Call {
                func,
                args: [dummy_spanned(arg)].into(),
                destination: get_temp_local(tcx, body),
                target: Some(BasicBlock::ZERO),
                unwind,
                call_source: CallSource::Misc,
                fn_span: DUMMY_SP,
            },
        };
        call_bbs.push(body.basic_blocks_mut().push(BasicBlockData::new(Some(terminator), false)));
    }

    // Redirect the FFI call's target to the first post block.
    if let TerminatorKind::Call { target, .. } = &mut body[call_block].terminator_mut().kind {
        *target = Some(call_bbs[0]);
    }

    for (idx, &bb) in call_bbs.iter().enumerate() {
        let target = call_bbs.get(idx + 1).copied().unwrap_or(final_target);
        body[bb].terminator_mut().successors_mut(|succ| *succ = target);
    }
}

fn instrument_ffi_boundary<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    let num_blocks = body.basic_blocks.next_index();

    // Pass 1: R1 (raw-ptr argument read/write) and R4 (locked API annotation).
    for current in START_BLOCK..num_blocks {
        let ffi = if let TerminatorKind::Call { func, .. } = &body[current].terminator().kind {
            is_ffi_call(tcx, body, func)
        } else {
            false
        };
        if ffi {
            instrument_ffi_call(tcx, body, current);
        }
    }

    // Pass 2: R2 (raw-pointer dereference read/write).
    for current in START_BLOCK..num_blocks {
        instrument_derefs(tcx, body, current);
    }
}

fn instrument_ffi_call<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>, current: BasicBlock) {
    let TerminatorKind::Call { func, args, .. } = body[current].terminator().kind.clone() else {
        return;
    };

    let mut accesses: Vec<Instr<'tcx>> = Vec::new();
    for arg in args.iter() {
        if let Operand::Copy(place) | Operand::Move(place) = &arg.node {
            let base = Place { local: place.local, projection: ty::List::empty() };
            if let ty::RawPtr(pointee, mutbl) = base.ty(&body.local_decls, tcx).ty.kind() {
                let want_mut = *mutbl == ty::Mutability::Mut;
                accesses.push(Instr {
                    wrapper: if want_mut { "ffi_race_write" } else { "ffi_race_read" },
                    pointee: *pointee,
                    arg: Operand::Copy(base),
                    want_mut,
                });
            }
        }
    }

    if accesses.is_empty() {
        return;
    }

    let locked = func
        .const_fn_def()
        .map(|(def_id, _)| def_id)
        .is_some_and(|def_id| is_known_locked_api(tcx, def_id));

    if !locked {
        insert_before_term(tcx, body, current, &accesses, Vec::new());
        return;
    }

    // R4: use the first raw-pointer argument as the mutex identity.
    let base = Instr {
        wrapper: "ffi_race_mutex_pre_lock",
        pointee: accesses[0].pointee,
        arg: accesses[0].arg.clone(),
        want_mut: false,
    };
    let post_lock = Instr { wrapper: "ffi_race_mutex_post_lock", ..base.clone() };
    let pre_unlock = Instr { wrapper: "ffi_race_mutex_pre_unlock", ..base.clone() };
    let post_unlock = Instr { wrapper: "ffi_race_mutex_post_unlock", ..base.clone() };

    // Flow: current -> pre_lock -> post_lock -> accesses -> FFI -> pre_unlock
    //           -> post_unlock -> original target.
    insert_after_call(tcx, body, current, &[pre_unlock, post_unlock]);

    let mut before = vec![base, post_lock];
    before.extend(accesses);
    insert_before_term(tcx, body, current, &before, Vec::new());
}

fn instrument_derefs<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>, current: BasicBlock) {
    let len = body[current].statements.len();
    for statement_index in (0..len).rev() {
        let loc = Location { block: current, statement_index };

        let mut visitor = DerefVisitor { vars: Vec::new() };
        visitor.visit_statement(&body[current].statements[statement_index], loc);
        if visitor.vars.is_empty() {
            continue;
        }

        let mut instrs: Vec<Instr<'tcx>> = Vec::new();
        for (base, is_write) in visitor.vars {
            let ty::RawPtr(pointee, mutbl) = base.ty(&body.local_decls, tcx).ty.kind() else {
                continue;
            };
            let pointee = *pointee;
            // MVP: only instrument thin pointers to sized types.
            if !pointee.is_sized(tcx, ty::TypingEnv::fully_monomorphized()) {
                continue;
            }
            if is_write && *mutbl != ty::Mutability::Mut {
                continue;
            }
            instrs.push(Instr {
                wrapper: if is_write { "ffi_race_write" } else { "ffi_race_read" },
                pointee,
                arg: Operand::Copy(base),
                want_mut: is_write,
            });
        }

        if instrs.is_empty() {
            continue;
        }
        let tail = body[current].statements.split_off(statement_index);
        insert_before_term(tcx, body, current, &instrs, tail);
    }
}

fn instrument_spawn_handoff<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    let mut locals: Vec<(Local, Ty<'tcx>)> = Vec::new();
    for (local, decl) in body.local_decls.iter_enumerated() {
        if let ty::RawPtr(pointee, _) = decl.ty.kind()
            && matches!(body.local_kind(local), LocalKind::Arg | LocalKind::Temp)
        {
            locals.push((local, *pointee));
        }
    }
    if locals.is_empty() {
        return;
    }

    let num_blocks = body.basic_blocks.next_index();
    for current in START_BLOCK..num_blocks {
        let spawn = if let TerminatorKind::Call { func, .. } = &body[current].terminator().kind {
            is_spawn_call(tcx, func)
        } else {
            false
        };
        if !spawn {
            continue;
        }
        let instrs: Vec<Instr<'tcx>> = locals
            .iter()
            .map(|&(local, pointee)| Instr {
                wrapper: "ffi_race_release",
                pointee,
                arg: Operand::Copy(Place::from(local)),
                want_mut: false,
            })
            .collect();
        insert_before_term(tcx, body, current, &instrs, Vec::new());
    }
}

fn instrument_closure_acquire<'tcx>(tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
    let instrs: Vec<Instr<'tcx>> = raw_ptr_upvar_places(tcx, body)
        .into_iter()
        .map(|(place, pointee)| Instr {
            wrapper: "ffi_race_acquire",
            pointee,
            arg: Operand::Copy(place),
            want_mut: false,
        })
        .collect();
    if instrs.is_empty() {
        return;
    }
    let tail = body[START_BLOCK].statements.split_off(0);
    insert_before_term(tcx, body, START_BLOCK, &instrs, tail);
}

struct DerefVisitor<'tcx> {
    vars: Vec<(Place<'tcx>, bool)>,
}

impl<'tcx> Visitor<'tcx> for DerefVisitor<'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, _: Location) {
        let Some(p) = place.projection.first() else { return };
        if matches!(p, ProjectionElem::Deref) {
            let base = Place { local: place.local, projection: ty::List::empty() };
            self.vars.push((base, context.is_mutating_use()));
        }
    }
}

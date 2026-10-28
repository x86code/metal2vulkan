//! MSL 3.2 program-scope bindings as parameters.
//!
//! MSL 3.2 lets a program-scope variable be bound like an argument (`constant float* constant
//! p [[buffer(1)]];`, `texture2d<half> t [[texture(9)]];`). `xcrun metal` compiles each to an
//! `externally_initialized` constant global with an `undef` initializer, named by an
//! `!air.global_binding` node whose argument node is an entry parameter's of index -1
//! ([`GlobalBinding`]), and reads it with a load of the value the argument holds: the buffer's
//! pointer, or the texture's handle (the only field of `metal::texture2d`, through a
//! `getelementptr` of indices 0, 0). Each such read is the value the entry's parameter of that
//! binding would hold, so `expand` adds to the entry one parameter per binding and type read this
//! way (two globals of one binding share it: Apple's compiler duplicates a global per translation
//! unit) and makes each load a read of it, as `global_builtin` does for built-ins. Lowering then
//! binds the parameter as every parameter of its role, at the binding's location index. A binding
//! not allowed in the entry's stage, or a global read another way, is left as it is.

use crate::abi::{AirAbi, GlobalBinding, ParamBinding, Stage};
use crate::budget::Meter;
use crate::error::Error;
use crate::ir::{ArgIdx, Body, FuncId, InstId, InstKind, ModuleParts, Type, TypeId, Value};

use super::const_loads::field_address;
use super::rewrite::{append_params, apply, invariant, scoped, BodyEdit};

/// The binding a load of `body` reads whole, when `kind` is one: a non-volatile, non-atomic load
/// of a pointer at the address of a bound global or of its first leaf at indices 0.
fn read<'a>(
    body: &Body,
    parts: &ModuleParts,
    abi: &'a AirAbi,
    kind: &InstKind,
    ty: TypeId,
) -> Option<&'a GlobalBinding> {
    let InstKind::Load {
        ptr,
        volatile: false,
        atomic: None,
        ..
    } = *kind
    else {
        return None;
    };
    if !matches!(parts.types.get(ty), Some(Type::Ptr { .. })) {
        return None;
    }
    let (g, path) = field_address(body, parts, ptr)?;
    if path.iter().any(|&i| i != 0) {
        return None;
    }
    abi.global_bindings.iter().find(|b| b.global == g)
}

/// Make each load of an MSL 3.2 program-scope binding in the `stage` entry `func`'s `body` a read
/// of a parameter added to the entry (see the module), returning the new body and the added
/// parameters' bindings (none, and `body` unchanged, when nothing reads one).
pub(super) fn expand(
    body: Body,
    parts: &mut ModuleParts,
    func: FuncId,
    abi: &AirAbi,
    stage: &Stage,
    m: &mut Meter,
) -> Result<(Body, Vec<ParamBinding>), Error> {
    if abi.global_bindings.is_empty() {
        return Ok((body, Vec::new()));
    }
    // The parameters to add, in the order the body first reads them: one per binding and type.
    let mut added: Vec<(&GlobalBinding, TypeId)> = Vec::new();
    let mut reads: Vec<(InstId, usize)> = Vec::new();
    for (i, inst) in body.insts.iter() {
        m.tick(1)?;
        let Some(b) = read(&body, parts, abi, &inst.kind, inst.ty) else {
            continue;
        };
        if !crate::abi::input_in_stage(&b.binding.role, stage) {
            continue;
        }
        let at = match added
            .iter()
            .position(|(a, t)| a.binding.role == b.binding.role && *t == inst.ty)
        {
            Some(at) => at,
            None => {
                m.reserve(&mut added, 1)?;
                added.push((b, inst.ty));
                added.len() - 1
            }
        };
        m.reserve(&mut reads, 1)?;
        reads.push((i, at));
    }
    let release = |m: &mut Meter, reads: &Vec<(InstId, usize)>, added: &Vec<_>| {
        m.release(reads.capacity() * std::mem::size_of::<(InstId, usize)>());
        m.release(added.capacity() * std::mem::size_of::<(&GlobalBinding, TypeId)>());
    };
    if added.is_empty() {
        release(m, &reads, &added);
        return Ok((body, Vec::new()));
    }
    let tys: Vec<TypeId> = added.iter().map(|&(_, t)| t).collect();
    let first = append_params(parts, func, &tys, m)?;
    let out = scoped(m, |m, held| {
        let mut edit = BodyEdit::keep_all(&body, m, held)?;
        for &(i, at) in &reads {
            m.tick(1)?;
            let p = u32::try_from(at)
                .ok()
                .and_then(|at| first.checked_add(at))
                .ok_or_else(|| invariant("entry parameters"))?;
            edit.replace(i, Value::Arg(ArgIdx::new(p)));
        }
        apply(&body, &edit, &|c| c, m)
    });
    let bindings = added
        .iter()
        .zip(first..)
        .map(|(&(b, _), param)| ParamBinding {
            param,
            ..b.binding.clone()
        })
        .collect();
    release(m, &reads, &added);
    let out = out?;
    body.release(m);
    Ok((out, bindings))
}

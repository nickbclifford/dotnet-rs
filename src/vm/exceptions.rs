use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    ops::Range,
};

use crate::{
    utils::DebugStr,
    value::{ConcreteType, Context},
};
use dotnetdll::prelude::*;

#[derive(Clone)]
pub struct ProtectedSection {
    pub instructions: Range<usize>,
    pub handlers: Vec<Handler>,
}
impl Debug for ProtectedSection {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_set()
            .entry(&DebugStr(format!("try {{ {:?} }}", self.instructions)))
            .entries(self.handlers.iter())
            .finish()
    }
}

#[derive(Clone)]
pub struct Handler {
    pub instructions: Range<usize>,
    pub kind: HandlerKind,
}
impl Debug for Handler {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} {{ {:?} }}", self.kind, self.instructions)
    }
}

#[derive(Clone)]
pub enum HandlerKind {
    // only triggers when exceptions are of this type
    Catch(ConcreteType),
    // only triggers when the filter clause pointed to returns true
    Filter { clause_offset: usize },
    // always triggers
    Finally,
    // triggers on any exception
    Fault,
}
impl Debug for HandlerKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use HandlerKind::*;
        match self {
            Catch(t) => write!(f, "catch({t:?})"),
            Filter { clause_offset } => write!(f, "filter({clause_offset}..)"),
            Finally => write!(f, "finally"),
            Fault => write!(f, "fault"),
        }
    }
}

pub fn parse<'a>(
    source: impl IntoIterator<Item = &'a body::Exception>,
    ctx: Context,
) -> Vec<ProtectedSection> {
    let mut sections = HashMap::new();
    for exc in source.into_iter() {
        use body::ExceptionKind::*;
        sections
            .entry(exc.try_offset..exc.try_offset + exc.try_length)
            .or_insert_with(Vec::new)
            .push(Handler {
                instructions: exc.handler_offset..exc.handler_offset + exc.handler_length,
                kind: match &exc.kind {
                    TypedException(t) => HandlerKind::Catch(ctx.make_concrete(t)),
                    Filter { offset } => HandlerKind::Filter {
                        clause_offset: *offset,
                    },
                    Finally => HandlerKind::Finally,
                    Fault => HandlerKind::Fault,
                },
            });
    }

    let mut v: Vec<_> = sections
        .into_iter()
        .map(|(k, v)| ProtectedSection {
            instructions: k,
            handlers: v,
        })
        .collect();
    v.sort_by_key(|s| (s.instructions.start, s.instructions.end));
    v
}

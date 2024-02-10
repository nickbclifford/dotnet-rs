use std::collections::HashMap;
use dotnetdll::prelude::*;
use crate::value::TypeDescription;
use enum_dispatch::enum_dispatch;

#[enum_dispatch]
pub trait HasLayout {
    fn size(&self) -> usize;
}

#[enum_dispatch(HasLayout)]
#[derive(Clone, Debug)]
pub enum LayoutManager {
    ClassLayoutManager,
    ArrayLayoutManager,
    Scalar
}
impl LayoutManager {
    pub fn is_gc_ptr(&self) -> bool {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => true,
            _ => false
        }
    }
}

#[derive(Clone, Debug)]
pub struct FieldLayout {
    pub position: usize,
    pub layout: LayoutManager
}
#[derive(Clone, Debug)]
pub struct ClassLayoutManager {
    pub fields: HashMap<String, FieldLayout>
}
impl HasLayout for ClassLayoutManager {
    fn size(&self) -> usize {
        self.fields.values().map(|f| f.layout.size()).sum()
    }
}
impl ClassLayoutManager {
    pub fn new(description: TypeDescription) -> Self {
        let layout = description.0.flags.layout;
        let fields: Vec<_> = description
            .0
            .fields
            .iter()
            .map(|f| (f.name.as_ref(), f.offset, type_layout(&f.return_type).size()))
            .collect();

        match layout {
            Layout::Automatic => {}
            Layout::Sequential(_) => {}
            Layout::Explicit(_) => {}
        }
        todo!()
    }

    pub fn field_offset(&self, name: &str) -> usize {
        match self.fields.get(name) {
            Some(l) => l.position,
            None => todo!("field not present in class")
        }
    }
}

#[derive(Clone, Debug)]
pub struct ArrayLayoutManager {
    pub element_layout: Box<LayoutManager>,
    pub length: usize
}
impl HasLayout for ArrayLayoutManager {
    fn size(&self) -> usize {
        self.element_layout.size() * self.length
    }
}
impl ArrayLayoutManager {
    pub fn new(element: &MemberType, length: usize) -> Self {
        Self {
            element_layout: Box::new(type_layout(element)),
            length,
        }
    }

    pub fn element_offset(&self, index: usize) -> usize {
        self.element_layout.size() * index
    }
}

#[derive(Clone, Debug)]
pub enum Scalar {
    ObjectRef
    // TODO: int/ptr types
}
impl HasLayout for Scalar {
    fn size(&self) -> usize {
        todo!()
    }
}

pub fn type_layout(r#type: &MemberType) -> LayoutManager {
    todo!()
}

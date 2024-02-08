use std::collections::HashMap;
use dotnetdll::prelude::*;
use crate::value::TypeDescription;
use enum_dispatch::enum_dispatch;

#[enum_dispatch]
pub trait HasLayout {
    fn size(&self) -> usize;
}

// TODO: scalars
#[enum_dispatch(HasLayout)]
pub enum LayoutManager {
    ClassLayoutManager,
    ArrayLayoutManager,
}

struct FieldLayout {
    position: usize,
    size: usize,
}
struct ClassLayoutManager {
    fields: HashMap<String, FieldLayout>
}
impl HasLayout for ClassLayoutManager {
    fn size(&self) -> usize {
        self.fields.values().map(|l| l.size).sum()
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

struct ArrayLayoutManager {
    element_size: usize,
    length: usize
}
impl HasLayout for ArrayLayoutManager {
    fn size(&self) -> usize {
        self.element_size * self.length
    }
}
impl ArrayLayoutManager {
    pub fn new(element: &MemberType, length: usize) -> Self {
        Self {
            element_size: type_layout(element).size(),
            length,
        }
    }

    pub fn element_offset(&self, index: usize) -> usize {
        self.element_size * index
    }
}

pub fn type_layout(r#type: &MemberType) -> LayoutManager {
    todo!()
}

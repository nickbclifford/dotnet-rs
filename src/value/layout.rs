use crate::value::TypeDescription;

// TODO
pub struct LayoutManager;

impl LayoutManager {
    pub fn user_type(description: TypeDescription) -> Self {
        let layout = description.0.flags.layout;
        let fields: Vec<_> = description
            .0
            .fields
            .iter()
            .map(|f| (f.name.as_ref(), f.offset))
            .collect();
        todo!()
    }

    pub fn array(element: TypeDescription, length: usize) -> Self {
        todo!()
    }
}

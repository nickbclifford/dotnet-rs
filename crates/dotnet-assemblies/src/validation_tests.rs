#[cfg(test)]
mod tests {
    use crate::validation::validate_metadata;
    use dotnetdll::prelude::*;
    use dotnetdll::resolved::{Accessibility, members::Method, signature::ReturnType, types::TypeDefinition};
    use dotnetdll::binary::signature::kinds::CallingConvention;

    #[test]
    fn test_valid_metadata() {
        let mut res = Resolution::new(Module::new("Test.dll"));
        res.push_type_definition(TypeDefinition::new(None, "TestType"));
        assert!(validate_metadata(&res).is_ok());
    }

    #[test]
    fn test_invalid_module_name() {
        let mut res = Resolution::new(Module::new(""));
        res.push_type_definition(TypeDefinition::new(None, "TestType"));
        let err = validate_metadata(&res).unwrap_err();
        assert!(err.to_string().contains("Module name is empty"));
    }

    #[test]
    fn test_empty_typedef_table() {
        let mut res = Resolution::new(Module::new("Test.dll"));
        // Resolution::new adds <Module> by default, so we clear it.
        res.type_definitions.clear();
        let err = validate_metadata(&res).unwrap_err();
        assert!(err.to_string().contains("TypeDef table is empty"));
    }

    #[test]
    fn test_self_inheritance() {
        let mut res = Resolution::new(Module::new("Test.dll"));
        let td = TypeDefinition::new(None, "SelfInherit");
        let type_idx = res.push_type_definition(td);
        
        // SelfInherit is at index 1 in the Vec.
        res.type_definitions[1].extends = Some(TypeSource::User(UserType::Definition(type_idx)));

        let err = validate_metadata(&res).unwrap_err();
        assert!(err.to_string().contains("extends itself"));
    }

    #[test]
    fn test_empty_method_name() {
        let mut res = Resolution::new(Module::new("Test.dll"));
        let td = TypeDefinition::new(None, "TestType");
        let type_idx = res.push_type_definition(td);
        
        let void_sig = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };
        
        res.push_method(type_idx, Method::new(Accessibility::Public, void_sig, "", None));

        let err = validate_metadata(&res).unwrap_err();
        assert!(err.to_string().contains("method at index 0 with empty name"));
    }
}

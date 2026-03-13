using System;

public class ClassConstraint<T> where T : class { }
public class StructConstraint<T> where T : struct { }
public class NewConstraint<T> where T : new() { }
public interface IInterface { }
public class InterfaceConstraint<T> where T : IInterface { }
public class BaseClass { }
public class BaseClassConstraint<T> where T : BaseClass { }

public class NoDefaultCtor {
    public NoDefaultCtor(int x) { }
}

public class Program {
    public static int Main() {
        // These will compile, but should fail at runtime when MakeGenericType is called
        // if the VM validates constraints.
        try { FailClass(); return 1; } catch (Exception) { }
        try { FailStruct(); return 1; } catch (Exception) { }
        try { FailNew(); return 1; } catch (Exception) { }
        try { FailInterface(); return 1; } catch (Exception) { }
        try { FailBase(); return 1; } catch (Exception) { }
        
        return 0; // Success if all caught
    }
    
    public static void FailClass() {
        typeof(ClassConstraint<>).MakeGenericType(typeof(int));
    }
    
    public static void FailStruct() {
        typeof(StructConstraint<>).MakeGenericType(typeof(string));
    }
    
    public static void FailNew() {
        typeof(NewConstraint<>).MakeGenericType(typeof(NoDefaultCtor));
    }
    
    public static void FailInterface() {
        typeof(InterfaceConstraint<>).MakeGenericType(typeof(int));
    }
    
    public static void FailBase() {
        typeof(BaseClassConstraint<>).MakeGenericType(typeof(int));
    }
}

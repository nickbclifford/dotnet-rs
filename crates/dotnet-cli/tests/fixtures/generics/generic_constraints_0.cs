using System;

public class ClassConstraint<T> where T : class { }
public class StructConstraint<T> where T : struct { }
public class NewConstraint<T> where T : new() { }
public interface IInterface { }
public class InterfaceConstraint<T> where T : IInterface { }
public class BaseClass { }
public class BaseClassConstraint<T> where T : BaseClass { }

public class Implementer : IInterface { }
public class Inheritor : BaseClass { }

public class Program {
    public static int Main() {
        // These should pass
        new ClassConstraint<string>();
        new StructConstraint<int>();
        new NewConstraint<Program>();
        new InterfaceConstraint<Implementer>();
        new BaseClassConstraint<Inheritor>();
        
        return 0;
    }
}

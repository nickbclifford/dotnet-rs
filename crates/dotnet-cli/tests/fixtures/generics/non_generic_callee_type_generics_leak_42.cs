using System;

// A callee declared on a non-generic type must not inherit the caller's type generics: the static
// state of Bar is a single instance, however many Foo<T> instantiations call into it.
public class Bar {
    public static int Inits;
    public static int Value = ++Inits;
    public static int Counter;
    public static int Get() { return Value; }
    public static int Bump() { return ++Counter; }
    public int InstanceBump() { return ++Counter; }
}

public class Foo<T> {
    public static int ViaMethod() { return Bar.Get(); }
    public static int ViaField() { return Bar.Value; }
    public static int ViaBump() { return Bar.Bump(); }
    public static int ViaInstance() { return new Bar().InstanceBump(); }
}

public class Program {
    public static int Main() {
        Foo<int>.ViaMethod();
        Foo<long>.ViaMethod();
        Foo<string>.ViaMethod();
        Foo<int>.ViaField();
        Foo<long>.ViaField();
        if (Bar.Inits != 1) return 1;
        if (Foo<int>.ViaBump() != 1) return 2;
        if (Foo<long>.ViaBump() != 2) return 3;
        if (Foo<string>.ViaInstance() != 3) return 4;
        if (Foo<long>.ViaInstance() != 4) return 5;
        if (Bar.Counter != 4) return 6;
        return 42;
    }
}

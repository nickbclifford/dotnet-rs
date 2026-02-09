using System;

public abstract class Base {
    public abstract int GetValue();
}

public class Derived : Base {
    public override int GetValue() => 42;
}

public class Program {
    delegate int Getter();
    
    public static int Main() {
        Base b = new Derived();
        Getter g = b.GetValue;
        return g(); // Should return 42
    }
}

using System;

class SideEffect {
    public static int ran = 0;
}

struct NoBeforeFieldInitVT {
    public int dummy;
    static NoBeforeFieldInitVT() {
        SideEffect.ran = 42;
    }
    public void InstanceMethod() {}
}

class Program {
    public static int Main() {
        NoBeforeFieldInitVT vt = new NoBeforeFieldInitVT();
        // C# compiler uses initobj for new struct with no parameterless ctor.
        // initobj doesn't trigger .cctor.
        
        int resultBefore = SideEffect.ran;
        vt.InstanceMethod();
        int resultAfter = SideEffect.ran;
        
        if (resultBefore == 0 && resultAfter == 42) {
            return 42;
        } else {
            return 0;
        }
    }
}

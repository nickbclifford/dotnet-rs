using System;

class SideEffect {
    public static int ran = 0;
}

class NoBeforeFieldInit {
    static NoBeforeFieldInit() {
        SideEffect.ran = 1;
    }
    public static int IsCctorRan() {
        return SideEffect.ran;
    }
}

class Program {
    public static int Main() {
        int result = NoBeforeFieldInit.IsCctorRan();
        return result * 42;
    }
}

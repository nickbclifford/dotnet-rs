using System;

public class Base {
    public int a;
}
public class Derived : Base {
    public int b;
}

public class Program {
    public static int Main() {
        // Exercise VMT cache
        for (int i = 0; i < 500; i++) {
            RunVirtual(new Derived());
        }

        // Exercise Hierarchy cache
        object d = new Derived();
        for (int i = 0; i < 100; i++) {
            if (!(d is Base)) return 1;
        }

        // Exercise Layout cache (indirectly through many allocations/checks)
        // Actually, just calling a method that needs to resolve types multiple times might help.
        
        return 0;
    }

    public static void RunVirtual(Base b) {
        b.ToString(); // ToString is virtual
    }
}

namespace Statics;

public class A {
    public static int x;
    static A() {
        // Accessing B.y will trigger B's .cctor
        x = B.y + 1;
    }
}

public class B {
    public static int y;
    static B() {
        // Accessing A.x will trigger A's .cctor (recursive)
        y = A.x + 1;
    }
}

public class Program {
    public static int Main() {
        // Trigger A's .cctor
        int val = A.x;
        
        // Expected behavior:
        // 1. A.cctor starts (A is INITIALIZING)
        // 2. A.cctor calls B.y access
        // 3. B.cctor starts (B is INITIALIZING)
        // 4. B.cctor calls A.x access
        // 5. A is INITIALIZING on same thread, returns Recursive.
        // 6. B sees A.x as 0 (default).
        // 7. B.y becomes 1.
        // 8. B.cctor finishes.
        // 9. A sees B.y as 1.
        // 10. A.x becomes 2.
        // 11. A.cctor finishes.
        
        if (val == 2 && B.y == 1) {
            return 42;
        }
        
        // If it returns 1, A.x was not 2.
        // If it returns 2, B.y was not 1.
        if (val != 2) return 1;
        if (B.y != 1) return 2;
        
        return 0;
    }
}

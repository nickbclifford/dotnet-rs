using System;

public class Program {
    public static int Main() {
        try {
            A();
        } catch (Exception e) {
            if (e.StackTrace.Contains("at Program.B") && 
                e.StackTrace.Contains("at Program.A") && 
                e.StackTrace.Contains("at Program.Main")) {
                return 42;
            }
        }
        return 1;
    }

    public static void A() {
        B();
    }

    public static void B() {
        throw new Exception("Test");
    }
}

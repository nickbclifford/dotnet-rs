using System;

public class Program {
    static int counter = 0;
    public delegate void MyDelegate();

    public static int Main() {
        MyDelegate d = A;
        d += B;
        try {
            d();
        } catch (Exception) {
            // counter should be 1 because B never ran
            if (counter == 1) return 42;
        }
        return counter;
    }

    static void A() {
        counter++;
        throw new Exception("A");
    }

    static void B() {
        counter++;
    }
}

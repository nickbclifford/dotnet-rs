using System;

public struct Pair { public int A; public int B; }
public delegate Pair Factory(int value);
public class Program {
    static Pair First(int value) { Console.WriteLine("first " + value); return new Pair { A = value, B = 1 }; }
    static Pair Second(int value) { Console.WriteLine("second " + value); return new Pair { A = value, B = 2 }; }
    public static int Main() {
        Factory factory = First;
        factory += Second;
        Pair pair = factory(40);
        Console.WriteLine(pair.A + pair.B);
        return pair.A + pair.B == 42 ? 42 : 1;
    }
}

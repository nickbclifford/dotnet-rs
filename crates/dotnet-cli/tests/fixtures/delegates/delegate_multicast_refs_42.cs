using System;

public delegate void RefPair(ref int first, ref int second);
public class Program {
    static void First(ref int first, ref int second) {
        first += 10;
        Console.WriteLine(second);
        GC.Collect();
    }
    static void Second(ref int first, ref int second) {
        second += 11;
        Console.WriteLine(first);
    }
    public static int Main() {
        RefPair targets = First;
        targets += Second;
        int local = 0;
        targets(ref local, ref local);
        int[] values = new int[1];
        targets(ref values[0], ref values[0]);
        return local + values[0] == 42 ? 42 : 1;
    }
}

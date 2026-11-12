using System;
using System.Threading;

public class Program {
    static int ready, done;
    static void Collect() {
        Volatile.Write(ref ready, 1);
        while (Volatile.Read(ref done) == 0) { GC.Collect(); Thread.Sleep(1); }
    }
    public static int Main() {
        const int length = 1024 * 1024 + 11;
        string a = new string(' ', length);
        string b = new string(' ', length);
        string different = new string(' ', length - 1) + "x";
        char[] zeros = new char[length];
        string zeroString = new string('\0', length);
        var collector = new Thread(Collect);
        collector.Start();
        while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
        bool ok = true;
        for (int i = 0; i < 2; i++) {
            string repeated = new string(' ', length);
            ok &= string.Equals(a, repeated);
            ok &= string.Equals(zeroString, new string(zeros));
            ok &= string.Equals(a, b) && a.Equals(b) && a.Equals((object)b);
            ok &= !string.Equals(a, different) && !a.Equals(different);
            ok &= string.IsNullOrWhiteSpace(a) && !string.IsNullOrWhiteSpace(different);
        }
        Volatile.Write(ref done, 1);
        collector.Join();
        Console.WriteLine(ok ? "equality ok" : "equality failed");
        return ok ? 42 : 1;
    }
}

using System;
using System.Threading;

public class Program {
    const int Length = 3 * 1024 * 1024 + 123;
    static int ready;
    static int done;

    static void Collect() {
        Volatile.Write(ref ready, 1);
        while (Volatile.Read(ref done) == 0) GC.Collect();
    }

    static bool Check(byte[] data, byte expected) {
        for (int i = 0; i < Length; i += 4099)
            if (data[i] != expected) return false;
        return data[0] == expected && data[Length - 1] == expected;
    }

    public static unsafe int Main() {
        byte[] data = new byte[Length];
        Thread collector = new Thread(Collect);
        collector.Start();
        while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
        bool ok = true;
        for (int repeat = 0; repeat < 8 && ok; repeat++) {
            byte value = (byte)(0x40 + repeat);
            new Span<byte>(data).Fill(value);
            ok = Check(data, value);
            if (ok) {
                new Span<byte>(data).Fill(0);
                ok = Check(data, 0);
            }
        }
        fixed (byte* p = data) {
            new Span<byte>(p, Length).Fill(7);
            if (p[0] != 7 || p[Length - 1] != 7 || p[Length / 2] != 7) ok = false;
        }
        Volatile.Write(ref done, 1);
        collector.Join();
        Console.WriteLine(ok ? "fill ok" : "fill corrupted");
        return ok ? 42 : 1;
    }
}

using System;
using System.Threading;

public class Program {
    const int Length = 2 * 1024 * 1024 + 8192;
    const int Shift = 4096;
    static int ready;
    static int done;

    static void Collect() {
        Volatile.Write(ref ready, 1);
        while (Volatile.Read(ref done) == 0) GC.Collect();
    }

    public static unsafe int Main() {
        byte[] original = new byte[Length + Shift];
        byte[] data = new byte[Length + Shift];
        for (int i = 0; i < original.Length; i++)
            original[i] = (byte)((i / Shift) % 251);

        Thread collector = new Thread(Collect);
        collector.Start();
        while (Volatile.Read(ref ready) == 0) Thread.Sleep(1);
        bool ok = true;
        fixed (byte* src = original, dst = data) {
            for (int repeat = 0; repeat < 16 && ok; repeat++) {
                Buffer.MemoryCopy(src, dst, data.Length, data.Length);
                Buffer.MemoryCopy(dst, dst + Shift, Length, Length);
                for (int i = 0; i < Length; i += Shift) {
                    if (data[i + Shift] != original[i]) { ok = false; break; }
                }
                Buffer.MemoryCopy(src, dst, data.Length, data.Length);
                Buffer.MemoryCopy(dst + Shift, dst, Length, Length);
                for (int i = 0; i < Length; i += Shift) {
                    if (data[i] != original[i + Shift]) { ok = false; break; }
                }
            }
        }
        Volatile.Write(ref done, 1);
        collector.Join();
        Console.WriteLine(ok ? "overlap preserved" : "overlap corrupted");
        return ok ? 42 : 1;
    }
}

using System;
using System.Diagnostics;

public struct Pair { public int A; public string B; }

public class Program {
    public static int Main() {
        // Large byte clear must reach SpanHelpers/initblk, not a per-element IL loop.
        byte[] data = new byte[3 * 1024 * 1024 + 5];
        data[0] = 1;
        data[data.Length / 2] = 2;
        data[data.Length - 1] = 3;
        Stopwatch sw = Stopwatch.StartNew();
        new Span<byte>(data).Clear();
        bool fast = sw.ElapsedMilliseconds < 3000;
        bool ok = data[0] == 0 && data[data.Length / 2] == 0 && data[data.Length - 1] == 0;

        int[] ints = new int[1000];
        for (int i = 0; i < ints.Length; i++) ints[i] = i + 1;
        new Span<int>(ints, 10, 500).Clear();
        ok &= ints[9] == 10 && ints[10] == 0 && ints[509] == 0 && ints[510] == 511;

        string[] strs = new string[100];
        for (int i = 0; i < strs.Length; i++) strs[i] = "x" + i;
        new Span<string>(strs, 5, 50).Clear();
        ok &= strs[4] == "x4" && strs[5] == null && strs[54] == null && strs[55] == "x55";

        Pair[] pairs = new Pair[10];
        for (int i = 0; i < pairs.Length; i++) { pairs[i].A = i + 1; pairs[i].B = "s" + i; }
        new Span<Pair>(pairs, 2, 5).Clear();
        ok &= pairs[1].A == 2 && pairs[1].B == "s1" && pairs[2].A == 0 && pairs[2].B == null
            && pairs[6].B == null && pairs[7].B == "s7" && pairs[7].A == 8;

        new Span<byte>(data, 0, 0).Clear();
        Console.WriteLine(ok ? "clear ok" : "clear wrong");
        Console.WriteLine(fast ? "clear fast" : "clear slow");
        return ok && fast ? 42 : 1;
    }
}

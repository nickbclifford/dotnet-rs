using System;
using System.Runtime.CompilerServices;

// Finalizers that allocate drive the collector while other finalizable objects are still queued.
// Every queued object must survive until its finalizer runs, however collection is interleaved.
public class Finalizable {
    public static int Finalized;
    public static string Last = "";
    readonly int id;
    public Finalizable(int id) { this.id = id; }
    ~Finalizable() {
        Last = id.ToString() + "/" + new object().GetType().Name;
        Finalized++;
    }
}

public class Program {
    const int PerRound = 200;
    const int Rounds = 5;

    [MethodImpl(MethodImplOptions.NoInlining)]
    static void Allocate(int round) {
        for (int i = 0; i < PerRound; i++) new Finalizable(round * PerRound + i);
    }

    public static int Main() {
        for (int round = 0; round < Rounds; round++) {
            Allocate(round);
            for (int i = 0; i < 3; i++) {
                GC.Collect();
                GC.WaitForPendingFinalizers();
            }
        }
        Console.WriteLine(Finalizable.Finalized);
        return Finalizable.Finalized == PerRound * Rounds && Finalizable.Last.EndsWith("/Object") ? 42 : 1;
    }
}

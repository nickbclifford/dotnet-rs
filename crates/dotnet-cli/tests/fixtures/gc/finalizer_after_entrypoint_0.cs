using System;

public sealed class FinalizerAfterEntrypoint
{
    public FinalizerAfterEntrypoint()
    {
        Program.Constructed++;
    }

    ~FinalizerAfterEntrypoint()
    {
        Program.Finalized++;
        Program.ObservedPhase = Program.Phase;
    }
}

public static class Program
{
    public static int Constructed;
    public static int Finalized;
    public static int Phase;
    public static int ObservedPhase;

    public static void Main()
    {
        new FinalizerAfterEntrypoint();
        if (Constructed != 1 || Finalized != 0 || ObservedPhase != 0)
            return;
        Phase = 42;
        GC.Collect();
    }
}

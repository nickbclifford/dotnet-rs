using System;
using System.Runtime.CompilerServices;

public sealed class HandleTarget
{
}

public class Program
{
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static RuntimeTypeHandle AllocateHandle()
    {
        return typeof(HandleTarget).TypeHandle;
    }

    public static int Main()
    {
        RuntimeTypeHandle handle = AllocateHandle();

        GC.Collect();
        GC.WaitForPendingFinalizers();

        Type target = Type.GetTypeFromHandle(handle);
        if (target != typeof(HandleTarget)) return 1;
        if (target.Name != nameof(HandleTarget)) return 2;

        return 42;
    }
}

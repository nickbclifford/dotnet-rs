using System;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;

class Program
{
    [MethodImpl(MethodImplOptions.NoInlining)]
    static Func<int> Create()
    {
        var method = new DynamicMethod("Alive", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Ret);
        return (Func<int>)method.CreateDelegate(typeof(Func<int>));
    }

    static int Main()
    {
        var action = Create();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        Console.WriteLine("delegate:" + action());
        Console.WriteLine("method:" + action.Method.Name + ":" + (action.Method.DeclaringType == null));
        GC.KeepAlive(action);
        return 42;
    }
}

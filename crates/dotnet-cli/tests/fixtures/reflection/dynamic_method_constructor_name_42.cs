using System;
using System.Reflection.Emit;

public static class Program
{
    public static int Main()
    {
        var method = new DynamicMethod(".ctor", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Ret);
        var call = (Func<int>)method.CreateDelegate(typeof(Func<int>));
        Console.WriteLine(call());
        return call();
    }
}

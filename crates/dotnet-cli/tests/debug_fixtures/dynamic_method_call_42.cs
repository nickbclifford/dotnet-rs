using System;
using System.Reflection;
using System.Reflection.Emit;

// Plan 24 slice 4: the call operand is a DynamicScope token, not a metadata token.
public static class Program
{
    public static int Twice(int value) => value * 2;

    public static int Main()
    {
        var method = new DynamicMethod("Call", typeof(int), new[] { typeof(int) });
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Call, typeof(Program).GetMethod(nameof(Twice), BindingFlags.Public | BindingFlags.Static)!);
        il.Emit(OpCodes.Ret);

        var call = (Func<int, int>)method.CreateDelegate(typeof(Func<int, int>));
        int result = call(21);
        Console.WriteLine($"Call: {result}");
        return result == 42 ? 42 : 1;
    }
}

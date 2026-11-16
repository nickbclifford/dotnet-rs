using System;
using System.Reflection;
using System.Reflection.Emit;

// DynamicMethod: the call operand is a DynamicScope token, not a metadata token. The callee must
// be the loaded Program, not a second copy of this assembly: its static counter is shared.
public static class Program
{
    static int s_calls;
    public static int Twice(int value) { s_calls++; return value * 2; }

    public static int Main()
    {
        var method = new DynamicMethod("Call", typeof(int), new[] { typeof(int) });
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Call, typeof(Program).GetMethod(nameof(Twice), BindingFlags.Public | BindingFlags.Static)!);
        il.Emit(OpCodes.Ret);

        var call = (Func<int, int>)method.CreateDelegate(typeof(Func<int, int>));
        int result = call(21);
        Console.WriteLine($"Call: {result}; calls: {s_calls}");
        return result == 42 && s_calls == 1 ? 42 : 1;
    }
}

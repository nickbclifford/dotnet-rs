using System;
using System.Reflection.Emit;

class Program
{
    public static ref int Set(ref int value) { value = 42; return ref value; }
    static int Main()
    {
        var byref = typeof(int).MakeByRefType();
        var method = new DynamicMethod("ByRef", typeof(int), new[] { byref });
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Call, typeof(Program).GetMethod("Set")!);
        il.Emit(OpCodes.Ldind_I4);
        il.Emit(OpCodes.Ret);
        object[] args = { 0 };
        var result = (int)method.Invoke(null, args)!;
        Console.WriteLine($"{result} {args[0]}");
        return result == 42 && (int)args[0] == 42 ? 42 : 1;
    }
}

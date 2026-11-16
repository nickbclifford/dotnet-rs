using System;
using System.Reflection.Emit;
class Program
{
    public static int First(int value, __arglist) => value;
    public static int Normal(int value) => value;
    static int Main()
    {
        try
        {
            var invalid = new DynamicMethod("Invalid", typeof(void), Type.EmptyTypes);
            invalid.GetILGenerator().EmitCall(OpCodes.Call, typeof(Program).GetMethod(nameof(Normal))!, Type.EmptyTypes);
            return 1;
        }
        catch (InvalidOperationException) { Console.WriteLine("invalid convention"); }

        var method = new DynamicMethod("VarargCall", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Ldc_I4, 11);
        il.EmitCall(OpCodes.Call, typeof(Program).GetMethod(nameof(First))!, new[] { typeof(int) });
        il.Emit(OpCodes.Ret);
        try { return ((Func<int>)method.CreateDelegate(typeof(Func<int>)))(); }
        catch (InvalidProgramException) { Console.WriteLine("vararg unavailable"); return 42; }
    }
}

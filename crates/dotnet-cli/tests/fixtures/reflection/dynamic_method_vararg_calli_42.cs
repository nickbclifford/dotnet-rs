using System;
using System.Reflection;
using System.Reflection.Emit;
class Program
{
    public static int First(int value, __arglist) => value;
    static int Main()
    {
        var method = new DynamicMethod("Vararg", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Ldc_I4, 11);
        il.Emit(OpCodes.Ldftn, typeof(Program).GetMethod(nameof(First))!);
        il.EmitCalli(OpCodes.Calli, CallingConventions.VarArgs, typeof(int), new[] { typeof(int) }, new[] { typeof(int) });
        il.Emit(OpCodes.Ret);
        try
        {
            return ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        }
        catch (InvalidProgramException)
        {
            Console.WriteLine("Vararg calling convention not supported");
            return 42;
        }
    }
}

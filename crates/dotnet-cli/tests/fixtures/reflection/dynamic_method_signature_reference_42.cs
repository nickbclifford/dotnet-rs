using System;
using System.Reflection;
using System.Reflection.Emit;

public class Box { public int Value = 42; }
class Program
{
    public static int Read(Box box) => box.Value;
    static int Main()
    {
        var method = new DynamicMethod("ReadBox", typeof(int), new[] { typeof(Box) });
        var il = method.GetILGenerator();
        var signature = SignatureHelper.GetMethodSigHelper(CallingConventions.Standard, typeof(int));
        signature.AddArgument(typeof(Box));
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Ldftn, typeof(Program).GetMethod(nameof(Read))!);
        il.Emit(OpCodes.Calli, signature);
        try { signature.AddArgument(typeof(int)); Console.WriteLine("mutable"); }
        catch (Exception e) { Console.WriteLine(e.GetType().Name); }
        il.Emit(OpCodes.Ret);
        int result = ((Func<Box, int>)method.CreateDelegate(typeof(Func<Box, int>)))(new Box());
        Console.WriteLine(result);
        return result;
    }
}

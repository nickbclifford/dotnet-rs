using System;
using System.Reflection;
using System.Reflection.Emit;

class Program
{
    public static int Twice(int value) => value * 2;

    static int Main()
    {
        try
        {
            var method = new DynamicMethod("PinnedSignature", typeof(int), Type.EmptyTypes);
            var il = method.GetILGenerator();
            var signature = SignatureHelper.GetMethodSigHelper(CallingConventions.Standard, typeof(int));
            signature.AddArgument(typeof(int), pinned: true);
            Console.WriteLine(Convert.ToHexString(signature.GetSignature()));
            il.Emit(OpCodes.Ldc_I4, 21);
            il.Emit(OpCodes.Ldftn, typeof(Program).GetMethod(nameof(Twice))!);
            il.Emit(OpCodes.Calli, signature);
            il.Emit(OpCodes.Ret);
            Console.WriteLine(((Func<int>)method.CreateDelegate(typeof(Func<int>)))());
        }
        catch (Exception exception)
        {
            Console.WriteLine(exception.GetType().Name);
        }
        return 42;
    }
}

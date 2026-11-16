using System;
using System.Reflection;
using System.Reflection.Emit;
class Program
{
    public static int FortyTwo() => 42;
    static void Probe(string name, Action<SignatureHelper> setup)
    {
        try
        {
            var method = new DynamicMethod(name, typeof(int), Type.EmptyTypes);
            var il = method.GetILGenerator();
            var signature = SignatureHelper.GetMethodSigHelper(CallingConventions.Standard, typeof(int));
            setup(signature);
            Console.Write(name + ":" + Convert.ToHexString(signature.GetSignature()) + ":");
            il.Emit(OpCodes.Ldftn, typeof(Program).GetMethod(nameof(FortyTwo))!);
            il.Emit(OpCodes.Calli, signature);
            il.Emit(OpCodes.Ret);
            Console.WriteLine(((Func<int>)method.CreateDelegate(typeof(Func<int>)))());
        }
        catch (Exception exception) { Console.WriteLine(exception.GetType().Name); }
    }
    static int Main()
    {
        Probe("trailing", s => s.AddSentinel());
        Probe("twice", s => { s.AddSentinel(); s.AddSentinel(); });
        return 42;
    }
}

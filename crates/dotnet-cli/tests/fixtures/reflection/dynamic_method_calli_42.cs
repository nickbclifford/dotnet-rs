using System;
using System.Reflection;
using System.Reflection.Emit;
class Program
{
    public static int Twice(int x) => x * 2;
    static int Main()
    {
        var method = new DynamicMethod("Indirect", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 21);
        il.Emit(OpCodes.Ldftn, typeof(Program).GetMethod(nameof(Twice))!);
        il.EmitCalli(OpCodes.Calli, CallingConventions.Standard, typeof(int), new[] { typeof(int) }, null);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

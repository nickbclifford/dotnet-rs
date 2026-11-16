using System;
using System.Reflection.Emit;

class Program
{
    public static T Identity<T>(T value) => value;
    static int Main()
    {
        var method = new DynamicMethod("Generic", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Call, typeof(Program).GetMethod("Identity")!.MakeGenericMethod(typeof(int)));
        il.Emit(OpCodes.Ret);
        var result = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(result);
        return result;
    }
}

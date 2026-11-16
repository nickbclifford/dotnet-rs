using System;
using System.Reflection.Emit;
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Switch", typeof(int), new[] { typeof(int) });
        var il = method.GetILGenerator();
        var first = il.DefineLabel();
        var second = il.DefineLabel();
        il.Emit(OpCodes.Ldarg_0);
        il.Emit(OpCodes.Switch, new[] { first, second });
        il.Emit(OpCodes.Ldc_I4_M1);
        il.Emit(OpCodes.Ret);
        il.MarkLabel(first);
        il.Emit(OpCodes.Ldc_I4_0);
        il.Emit(OpCodes.Ret);
        il.MarkLabel(second);
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Ret);
        var f = (Func<int, int>)method.CreateDelegate(typeof(Func<int, int>));
        int answer = f(1) + f(0) + f(4) + 1;
        Console.WriteLine(answer);
        return answer;
    }
}

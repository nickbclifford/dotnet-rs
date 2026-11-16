using System;
using System.Reflection.Emit;
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Array", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4_1);
        il.Emit(OpCodes.Newarr, typeof(int));
        il.Emit(OpCodes.Dup);
        il.Emit(OpCodes.Ldc_I4_0);
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Stelem_I4);
        il.Emit(OpCodes.Ldc_I4_0);
        il.Emit(OpCodes.Ldelem_I4);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

using System;
using System.Reflection.Emit;
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("DeepStack", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        for (int i = 0; i < 40; i++) il.Emit(OpCodes.Ldc_I4_1);
        for (int i = 1; i < 40; i++) il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Ldc_I4_2);
        il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

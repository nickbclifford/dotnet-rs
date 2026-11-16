using System;
using System.Reflection.Emit;
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Offset", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        var target = il.DefineLabel();
        il.Emit(OpCodes.Ldc_I4_0);
        il.Emit(OpCodes.Switch, new[] { target, target });
        il.Emit(OpCodes.Ldc_I4, 35);
        il.Emit(OpCodes.Ret);
        il.MarkLabel(target);
        int offset = il.ILOffset;
        il.Emit(OpCodes.Ldc_I4, 42 - offset);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))() + offset;
        Console.WriteLine(answer);
        return answer;
    }
}

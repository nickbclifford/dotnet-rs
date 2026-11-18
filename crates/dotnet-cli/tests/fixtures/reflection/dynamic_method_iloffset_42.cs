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
        // Pin the size of this instruction; with a two-target switch, the label is at
        // 1 (ldc.i4.0) + 13 (switch and its table) + 2 (ldc.i4.s) + 1 (ret) = 17.
        il.Emit(OpCodes.Ldc_I4_S, (sbyte)35);
        il.Emit(OpCodes.Ret);
        il.MarkLabel(target);
        int offset = il.ILOffset;
        if (offset != 17) return 1;
        il.Emit(OpCodes.Ldc_I4, 42 - offset);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))() + offset;
        Console.WriteLine(answer);
        return answer;
    }
}

using System;
using System.Reflection.Emit;
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("PinnedLocal", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        var local = il.DeclareLocal(typeof(object), true);
        il.Emit(OpCodes.Ldnull);
        il.Emit(OpCodes.Stloc, local);
        il.Emit(OpCodes.Ldc_I4, local.IsPinned ? 42 : 7);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

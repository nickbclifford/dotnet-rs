using System;
using System.Reflection.Emit;

class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Filter", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        var result = il.DeclareLocal(typeof(int));
        il.BeginExceptionBlock();
        il.Emit(OpCodes.Newobj, typeof(Exception).GetConstructor(Type.EmptyTypes)!);
        il.Emit(OpCodes.Throw);
        il.BeginExceptFilterBlock();
        il.Emit(OpCodes.Pop);
        il.Emit(OpCodes.Ldc_I4_1);
        il.BeginCatchBlock(null);
        il.Emit(OpCodes.Pop);
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Stloc, result);
        il.EndExceptionBlock();
        il.Emit(OpCodes.Ldloc, result);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

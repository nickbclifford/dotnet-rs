using System;
using System.Reflection.Emit;
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("CatchFinally", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        var result = il.DeclareLocal(typeof(int));
        il.BeginExceptionBlock();
        il.Emit(OpCodes.Newobj, typeof(Exception).GetConstructor(Type.EmptyTypes)!);
        il.Emit(OpCodes.Throw);
        il.BeginCatchBlock(typeof(Exception));
        il.Emit(OpCodes.Pop);
        il.Emit(OpCodes.Ldc_I4, 40);
        il.Emit(OpCodes.Stloc, result);
        il.BeginFinallyBlock();
        il.Emit(OpCodes.Ldloc, result);
        il.Emit(OpCodes.Ldc_I4_2);
        il.Emit(OpCodes.Add);
        il.Emit(OpCodes.Stloc, result);
        il.EndExceptionBlock();
        il.Emit(OpCodes.Ldloc, result);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

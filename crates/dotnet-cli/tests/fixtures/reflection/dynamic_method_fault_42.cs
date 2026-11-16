using System;
using System.Reflection.Emit;

public class FaultState { public static int Value; }
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("Fault", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.BeginExceptionBlock();
        il.BeginExceptionBlock();
        il.Emit(OpCodes.Newobj, typeof(Exception).GetConstructor(Type.EmptyTypes)!);
        il.Emit(OpCodes.Throw);
        il.BeginFaultBlock();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Stsfld, typeof(FaultState).GetField("Value")!);
        il.EndExceptionBlock();
        il.BeginCatchBlock(typeof(Exception));
        il.Emit(OpCodes.Pop);
        il.EndExceptionBlock();
        il.Emit(OpCodes.Ldsfld, typeof(FaultState).GetField("Value")!);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

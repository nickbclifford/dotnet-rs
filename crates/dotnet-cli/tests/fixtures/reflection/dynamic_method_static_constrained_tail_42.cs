using System;
using System.Reflection.Emit;
public interface IValue { static abstract int Get(); }
public struct Value : IValue { public static int Get() => 42; }
class Program
{
    static int Main()
    {
        var method = new DynamicMethod("StaticConstrainedTail", typeof(int), Type.EmptyTypes);
        var il = method.GetILGenerator();
        il.Emit(OpCodes.Constrained, typeof(Value));
        il.Emit(OpCodes.Tailcall);
        il.Emit(OpCodes.Call, typeof(IValue).GetMethod(nameof(IValue.Get))!);
        il.Emit(OpCodes.Ret);
        int answer = ((Func<int>)method.CreateDelegate(typeof(Func<int>)))();
        Console.WriteLine(answer);
        return answer;
    }
}

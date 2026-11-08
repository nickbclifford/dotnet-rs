using System;
using System.Reflection;
using System.Globalization;

public delegate int Work(int value);
public delegate object Reflect(object receiver, BindingFlags flags, Binder binder, object[] args, CultureInfo culture);
public static class Delayed {
    static Delayed() { Console.WriteLine("delayed cctor"); }
    public static int Call(int value) { Console.WriteLine("delayed call"); return value + 2; }
}
public static class DelayedVoid {
    static DelayedVoid() { Console.WriteLine("void cctor"); }
    public static void Call(int value) { Console.WriteLine("void call " + value); }
}
public class Program {
    static int First(int value) { Console.WriteLine("first"); return value + 1; }
    static int Second(int value) { Console.WriteLine("second"); return value + 2; }
    public static int Main() {
        Work work = First;
        work += Second;
        MethodInfo invoke = typeof(Work).GetMethod("Invoke");
        int result = (int)invoke.Invoke(work, new object[] { 40 });
        Func<string, string, bool> equals = string.Equals;
        equals += string.Equals;
        MethodInfo compare = typeof(Func<string, string, bool>).GetMethod("Invoke");
        bool same = (bool)compare.Invoke(equals, new object[] { "same", "same" });
        Func<object, object[], object> indirect = invoke.Invoke;
        indirect += invoke.Invoke;
        int nested = (int)indirect(work, new object[] { 40 });
        Reflect direct = invoke.Invoke;
        direct += invoke.Invoke;
        int directNested = (int)direct(work, BindingFlags.Default, null, new object[] { 40 }, null);
        MethodInfo delayedMethod = typeof(Delayed).GetMethod("Call");
        Reflect delayed = delayedMethod.Invoke;
        delayed += delayedMethod.Invoke;
        int delayedResult = (int)delayed(null, BindingFlags.Default, null, new object[] { 40 }, null);
        MethodInfo voidMethod = typeof(DelayedVoid).GetMethod("Call");
        Reflect voidCall = voidMethod.Invoke;
        voidCall += voidMethod.Invoke;
        object voidResult = voidCall(null, BindingFlags.Default, null, new object[] { 40 }, null);
        Console.WriteLine(result);
        Console.WriteLine(same);
        Console.WriteLine(nested);
        Console.WriteLine(directNested);
        Console.WriteLine(delayedResult);
        return result == 42 && same && nested == 42 && directNested == 42 && delayedResult == 42 && voidResult == null ? 42 : 1;
    }
}

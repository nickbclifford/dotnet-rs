using System;
using System.Reflection;
using System.Threading;

public struct Payload { public object Item; public int Number; }
public enum Choice : long { First = 20, Second = 22 }
public static class Program
{
    static void Pause() { Thread.Sleep(0); GC.Collect(); }
    public static Payload PayloadTarget(Payload value) { Pause(); value.Number += 2; return value; }
    public static int? NullableTarget(int? value) { Pause(); return value + 2; }
    public static DateTime DateTarget(DateTime value) { Pause(); return value.AddTicks(2); }
    public static Choice EnumTarget(Choice value) { Pause(); return Choice.Second; }
    public static void ByRefTarget(ref Payload value) { Pause(); value.Number += 2; }
    static object Invoke(string name, object value) => typeof(Program).GetMethod(name)!.Invoke(null, new object[] { value })!;
    public static int Main()
    {
        var marker = new object();
        var result = (Payload)Invoke(nameof(PayloadTarget), new Payload { Item = marker, Number = 40 });
        object[] byref = { new Payload { Item = marker, Number = 40 } };
        typeof(Program).GetMethod(nameof(ByRefTarget))!.Invoke(null, byref);
        var updated = (Payload)byref[0];
        bool ok = ReferenceEquals(result.Item, marker) && result.Number == 42
            && ReferenceEquals(updated.Item, marker) && updated.Number == 42
            && (int?)Invoke(nameof(NullableTarget), 40) == 42
            && Invoke(nameof(NullableTarget), null!) == null
            && ((DateTime)Invoke(nameof(DateTarget), new DateTime(40))).Ticks == 42
            && (Choice)Invoke(nameof(EnumTarget), Choice.First) == Choice.Second;
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}

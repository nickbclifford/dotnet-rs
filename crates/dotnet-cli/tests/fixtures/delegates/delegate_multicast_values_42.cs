using System;
using System.Threading;

public struct Payload { public object Item; public int Number; }
public enum Choice : long { First = 20, Second = 22 }
public delegate T Transform<T>(T value);

public static class Program
{
    static void Pause() { Thread.Sleep(0); GC.Collect(); }
    static Payload PayloadFirst(Payload value) { Pause(); return value; }
    static Payload PayloadSecond(Payload value) { Pause(); value.Number += 2; return value; }
    static int? NullableFirst(int? value) { Pause(); return value; }
    static int? NullableSecond(int? value) { Pause(); return value + 2; }
    static DateTime DateFirst(DateTime value) { Pause(); return value; }
    static DateTime DateSecond(DateTime value) { Pause(); return value.AddTicks(2); }
    static Choice EnumFirst(Choice value) { Pause(); return value; }
    static Choice EnumSecond(Choice value) { Pause(); return Choice.Second; }

    public static int Main()
    {
        Transform<Payload> payload = PayloadFirst; payload += PayloadSecond;
        Transform<int?> nullable = NullableFirst; nullable += NullableSecond;
        Transform<DateTime> date = DateFirst; date += DateSecond;
        Transform<Choice> choice = EnumFirst; choice += EnumSecond;
        var marker = new object();
        var result = payload(new Payload { Item = marker, Number = 40 });
        bool ok = ReferenceEquals(result.Item, marker) && result.Number == 42
            && nullable(40) == 42 && nullable(null) == null
            && date(new DateTime(40)).Ticks == 42 && choice(Choice.First) == Choice.Second;
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}

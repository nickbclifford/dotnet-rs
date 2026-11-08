using System;
using System.Reflection;

public struct Payload { public int Value; }
public static class FallbackTarget
{
    public static int Calls;
    static FallbackTarget() { Calls = 40; }
    public static Payload Make() { Calls++; return new Payload { Value = Calls }; }
    public static Payload Throw() { throw new InvalidOperationException(); }
}
public static class Log { public static int Initializers; }
public static class FailingTarget
{
    static FailingTarget() { Log.Initializers++; throw new InvalidOperationException(); }
    public static Payload Make() => new Payload { Value = 0 };
}
public static class Program
{
    public static bool Filter() => true;
    public static int Main()
    {
        var type = typeof(FallbackTarget);
        var make = type.GetMethod(nameof(FallbackTarget.Make))!;
        var result = (Payload)make.Invoke(null, null)!;
        if (result.Value != 41) return 1;
        try { type.GetMethod(nameof(FallbackTarget.Throw))!.Invoke(null, null); return 2; }
        catch (Exception) when (Filter()) { }
        var failing = typeof(FailingTarget).GetMethod(nameof(FailingTarget.Make))!;
        for (int index = 0; index < 2; index++)
        {
            try { failing.Invoke(null, null); return 3; }
            catch (Exception) { }
        }
        var second = (Payload)make.Invoke(null, null)!;
        return second.Value == 42 && Log.Initializers == 1 ? 42 : 4;
    }
}

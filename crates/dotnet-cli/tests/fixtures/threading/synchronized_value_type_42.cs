using System;
using System.Runtime.CompilerServices;

struct InstanceProbe
{
    [MethodImpl(MethodImplOptions.Synchronized)] public void Run() { }
}
struct StaticProbe
{
    [MethodImpl(MethodImplOptions.Synchronized)] public static void Run() { }
}
struct ValidProbe { public static int Run() => 42; }
class Program
{
    [MethodImpl(MethodImplOptions.NoInlining)] static void LoadInstance() { Console.WriteLine(typeof(InstanceProbe)); }
    [MethodImpl(MethodImplOptions.NoInlining)] static void LoadStatic() { Console.WriteLine(typeof(StaticProbe)); }
    [MethodImpl(MethodImplOptions.NoInlining)] static void CallInstance() { var p = new InstanceProbe(); p.Run(); }
    [MethodImpl(MethodImplOptions.NoInlining)] static void CallStatic() { StaticProbe.Run(); }
    [MethodImpl(MethodImplOptions.NoInlining)] static bool CatchInside()
    {
        try { Console.WriteLine(typeof(InstanceProbe)); return false; }
        catch (TypeLoadException) { return true; }
    }
    static bool Rejected(Action action)
    {
        try { action(); return false; }
        catch (TypeLoadException) { return true; }
    }
    public static int Main()
    {
        if (!Rejected(LoadInstance) || !Rejected(LoadStatic)) return 1;
        if (!Rejected(CallInstance) || !Rejected(CallStatic)) return 2;
        try { CatchInside(); return 3; }
        catch (TypeLoadException) { }
        return ValidProbe.Run();
    }
}

using System;
using System.Reflection;

public class Target
{
    public static readonly Exception Error = new InvalidOperationException();
    public Target() { throw Error; }
    public static void Throw() { throw Error; }
}

public class Program
{
    static bool Filter(Exception e) => e is TargetInvocationException && ReferenceEquals(e.InnerException, Target.Error);
    public static int Main()
    {
        var method = typeof(Target).GetMethod(nameof(Target.Throw))!;
        try { method.Invoke(null, null); return 1; }
        catch (Exception e) when (Filter(e)) { }
        try { method.Invoke(null, BindingFlags.DoNotWrapExceptions, null, null, null); return 2; }
        catch (InvalidOperationException e) { if (!ReferenceEquals(e, Target.Error)) return 3; }
        try { typeof(Target).GetConstructor(Type.EmptyTypes)!.Invoke(null); return 4; }
        catch (TargetInvocationException e) { if (!ReferenceEquals(e.InnerException, Target.Error)) return 5; }
        try { Activator.CreateInstance(typeof(Target), Array.Empty<object>()); return 6; }
        catch (TargetInvocationException e) { if (!ReferenceEquals(e.InnerException, Target.Error)) return 7; }
        try { Activator.CreateInstance<Target>(); return 8; }
        catch (TargetInvocationException e) { if (!ReferenceEquals(e.InnerException, Target.Error)) return 9; }
        try { Activator.CreateInstance(typeof(Target)); return 10; }
        catch (TargetInvocationException e) { if (!ReferenceEquals(e.InnerException, Target.Error)) return 11; }
        return 42;
    }
}

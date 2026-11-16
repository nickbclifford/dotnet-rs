using System;
using System.Collections.Generic;
using System.Reflection.Emit;

// DynamicMethod's owner-type constructor rejects interface, element and open generic owners before
// it looks up the owner's module. The interface check (RuntimeType.IsActualInterface) used to read
// RuntimeType.m_handle through GetNativeTypeHandle and raised AccessViolationException on the VM
// (plan 24 slice 2). See also reflection/dynamic_method_ctor_42.cs.
public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }
    static bool RejectsOwner(Type owner)
    {
        try
        {
            _ = new DynamicMethod("m", typeof(int), Type.EmptyTypes, owner, skipVisibility: true);
            return false;
        }
        catch (ArgumentException) { return true; }
        catch (Exception e) { Console.WriteLine(e.GetType().FullName); return false; }
    }
    public static int Main()
    {
        Check(1, RejectsOwner(typeof(IDisposable)));
        Check(2, RejectsOwner(typeof(IEnumerable<int>)));
        Check(3, RejectsOwner(typeof(int[])));
        Check(4, RejectsOwner(typeof(List<>)));
        Console.WriteLine(failures == 0 ? "ok" : "failures: " + failures);
        return failures == 0 ? 42 : 1;
    }
}

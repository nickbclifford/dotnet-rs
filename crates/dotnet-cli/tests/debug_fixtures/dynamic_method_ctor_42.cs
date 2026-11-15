using System;
using System.Reflection.Emit;

// Plan 24 slice 2: constructing a DynamicMethod. Expected to differ from `dotnet` until the VM
// supports DynamicMethod; keep it here, out of the auto-discovered fixtures, until it passes.
// Before 2026-11 this raised AccessViolationException from RuntimeType.GetNativeTypeHandle
// (DynamicMethod.Init → get_IsActualInterface). It now fails one step later, in
// DynamicMethod.GetDynamicMethodsModule → AssemblyLoadContext.get_Default → the
// InitializeAssemblyLoadContext QCall (TypeInitializationException / DllNotFoundException 'QCall').
public static class Program
{
    public static int Main()
    {
        try
        {
            var method = new DynamicMethod("m", typeof(int), Type.EmptyTypes);
            Console.WriteLine(method.Name);
            return 42;
        }
        catch (Exception e)
        {
            Console.WriteLine(e.GetType().FullName);
            return 1;
        }
    }
}

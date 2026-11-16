using System;
using System.Reflection.Emit;

// Constructing a DynamicMethod. Before plan 24 this raised AccessViolationException from
// RuntimeType.GetNativeTypeHandle (DynamicMethod.Init → get_IsActualInterface), then stopped at the
// InitializeAssemblyLoadContext QCall once the AV was fixed.
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

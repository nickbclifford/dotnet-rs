using System;
using System.Reflection;
using System.Reflection.Emit;
class Program
{
    static int Main()
    {
        var method = SignatureHelper.GetMethodSigHelper(CallingConventions.Standard, typeof(int));
        method.AddArgument(typeof(int[]));
        method.AddArgument(typeof(int), pinned: true);
        Console.WriteLine(Convert.ToHexString(method.GetSignature()));
        var locals = SignatureHelper.GetLocalVarSigHelper();
        locals.AddArgument(typeof(int));
        Console.WriteLine(Convert.ToHexString(locals.GetSignature()));
        var field = SignatureHelper.GetFieldSigHelper(null);
        field.AddArgument(typeof(long));
        Console.WriteLine(Convert.ToHexString(field.GetSignature()));
        var property = SignatureHelper.GetPropertySigHelper(null, typeof(int), new[] { typeof(int) });
        Console.WriteLine(Convert.ToHexString(property.GetSignature()));
        var propertyWithModifiers = SignatureHelper.GetPropertySigHelper(null, typeof(int), null, null,
            new[] { typeof(int) }, null, null);
        Console.WriteLine(Convert.ToHexString(propertyWithModifiers.GetSignature()));
        return 42;
    }
}

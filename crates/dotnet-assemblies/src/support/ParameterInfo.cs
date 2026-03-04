using System.Reflection;
using System.Runtime.CompilerServices;

namespace DotnetRs;

public class ParameterInfo : System.Reflection.ParameterInfo
{
    private nint method_index;
    private int position;

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string GetName();
    public override string? Name => GetName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Type GetParameterType();
    public override Type ParameterType => GetParameterType();

    public override int Position => position;
}

using System.Reflection;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

public class ParameterInfo : System.Reflection.ParameterInfo
{
    [UsedImplicitly] private nint method_index;
    [UsedImplicitly] private int position;

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string GetName();
    public override string? Name => GetName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Type GetParameterType();
    public override Type ParameterType => GetParameterType();

    public override int Position => position;
}

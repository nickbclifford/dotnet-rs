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

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern MemberInfo GetMember();
    public override MemberInfo Member => GetMember();

    public override object[] GetCustomAttributes(bool inherit) => System.Array.Empty<object>();

    public override object[] GetCustomAttributes(Type attributeType, bool inherit) =>
        System.Array.Empty<object>();

    public override System.Collections.Generic.IList<System.Reflection.CustomAttributeData> GetCustomAttributesData() =>
        new System.Collections.Generic.List<System.Reflection.CustomAttributeData>();

    public override bool IsDefined(Type attributeType, bool inherit) => false;
}

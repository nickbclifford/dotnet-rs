using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

public class MethodInfo : System.Reflection.MethodInfo
{
    [UsedImplicitly] private nint index;

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object[] GetCustomAttributes(bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object[] GetCustomAttributes(Type attributeType, bool inherit);

    public override System.Collections.Generic.IList<System.Reflection.CustomAttributeData> GetCustomAttributesData() =>
        new System.Collections.Generic.List<System.Reflection.CustomAttributeData>();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern ICustomAttributeProvider GetReturnTypeAttributes();
    public override ICustomAttributeProvider ReturnTypeCustomAttributes => GetReturnTypeAttributes();

    public override ParameterInfo ReturnParameter => new ReturnParameterStub(this);

    private sealed class ReturnParameterStub : ParameterInfo
    {
        private readonly MethodInfo owner;

        public ReturnParameterStub(MethodInfo owner)
        {
            this.owner = owner;
        }

        public override object[] GetCustomAttributes(bool inherit) => System.Array.Empty<object>();

        public override object[] GetCustomAttributes(Type attributeType, bool inherit) =>
            System.Array.Empty<object>();

        public override System.Collections.Generic.IList<System.Reflection.CustomAttributeData> GetCustomAttributesData() =>
            new System.Collections.Generic.List<System.Reflection.CustomAttributeData>();

        public override bool IsDefined(Type attributeType, bool inherit) => false;

        public override MemberInfo Member => owner;

        public override int Position => -1;

        public override Type ParameterType => owner.ReturnType;

        public override string? Name => null;
    }

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern bool IsDefined(Type attributeType, bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Type? GetDeclaringType();
    public override Type? DeclaringType => GetDeclaringType();
    public override Type? ReflectedType => GetDeclaringType();
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string GetName();
    public override string Name => GetName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern MethodImplAttributes GetMethodImplementationFlags();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern ParameterInfo[] GetParameters();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object? Invoke(object? obj, BindingFlags invokeAttr, Binder? binder, object?[]? parameters,
        CultureInfo? culture);

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern MethodAttributes GetMethodFlags();
    public override MethodAttributes Attributes => GetMethodFlags();
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern System.RuntimeMethodHandle GetMethodHandle();
    public override System.RuntimeMethodHandle MethodHandle => GetMethodHandle();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern System.Reflection.MethodInfo GetBaseDefinition();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type[] GetGenericArguments();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern System.Reflection.MethodInfo GetGenericMethodDefinition();
}
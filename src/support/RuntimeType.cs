using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;

namespace DotnetRs;

[Stub(InPlaceOf = "System.RuntimeType")]
internal class RuntimeType : Type
{
    private nint concreteType;
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Assembly GetAssembly();
    public override System.Reflection.Assembly Assembly => GetAssembly();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string? GetQualifiedName();
    public override string? AssemblyQualifiedName => GetQualifiedName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Type? GetBaseType();
    public override Type? BaseType => GetBaseType();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string GetName();
    public override string Name => GetName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string? GetFullName();
    public override string? FullName => GetFullName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Module GetModule();
    public override System.Reflection.Module Module => GetModule();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string? GetNamespace();
    public override string? Namespace => GetNamespace();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern System.RuntimeTypeHandle GetTypeHandle();
    public override System.RuntimeTypeHandle TypeHandle => GetTypeHandle();

    // shrug
    public override Type UnderlyingSystemType => this;


    [MethodImpl(MethodImplOptions.InternalCall)]
    protected override extern TypeAttributes GetAttributeFlagsImpl();

    [MethodImpl(MethodImplOptions.InternalCall)]
    protected override extern ConstructorInfo? GetConstructorImpl(
        BindingFlags bindingAttr,
        Binder? binder,
        CallingConventions callConvention,
        Type[] types,
        ParameterModifier[]? modifiers
    );

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern ConstructorInfo[] GetConstructors(BindingFlags bindingAttr);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern MemberInfo[] GetMembers(BindingFlags bindingAttr);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern EventInfo? GetEvent(string name, BindingFlags bindingAttr);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern EventInfo[] GetEvents(BindingFlags bindingAttr);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern FieldInfo? GetField(string name, BindingFlags bindingAttr);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern FieldInfo[] GetFields(BindingFlags bindingAttr);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type? GetInterface(string name, bool ignoreCase);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type[] GetInterfaces();

    [MethodImpl(MethodImplOptions.InternalCall)]
    protected override extern System.Reflection.MethodInfo? GetMethodImpl(
        string name,
        BindingFlags bindingAttr,
        Binder? binder,
        CallingConventions callConvention,
        Type[]? types,
        ParameterModifier[]? modifiers
    );

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern System.Reflection.MethodInfo[] GetMethods(BindingFlags bindingAttr);
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type? GetNestedType(string name, BindingFlags bindingAttr);
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type[] GetNestedTypes(BindingFlags bindingAttr);
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern PropertyInfo[] GetProperties(BindingFlags bindingAttr);
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    protected override extern PropertyInfo? GetPropertyImpl(
        string name,
        BindingFlags bindingAttr,
        Binder? binder,
        Type? returnType,
        Type[]? types,
        ParameterModifier[]? modifiers
    );

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern byte[] GetGuid();
    public override Guid GUID => new (GetGuid());

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object? InvokeMember(
        string name,
        BindingFlags invokeAttr,
        Binder? binder,
        object? target,
        object?[]? args,
        ParameterModifier[]? modifiers,
        CultureInfo? culture,
        string[]? namedParameters
    );

    [MethodImpl(MethodImplOptions.InternalCall)]
    protected override extern bool IsArrayImpl();

    [MethodImpl(MethodImplOptions.InternalCall)]    
    protected override extern bool IsByRefImpl();

    [MethodImpl(MethodImplOptions.InternalCall)]    
    protected override extern bool IsCOMObjectImpl();

    [MethodImpl(MethodImplOptions.InternalCall)]    
    protected override extern bool IsPointerImpl();

    [MethodImpl(MethodImplOptions.InternalCall)]    
    protected override extern bool IsPrimitiveImpl();


    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type? GetElementType();

    [MethodImpl(MethodImplOptions.InternalCall)]    
    protected override extern bool HasElementTypeImpl();

    [MethodImpl(MethodImplOptions.InternalCall)]    
    public override extern object[] GetCustomAttributes(bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]    
    public override extern object[] GetCustomAttributes(Type attributeType, bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]    
    public override extern bool IsDefined(Type attributeType, bool inherit);
}
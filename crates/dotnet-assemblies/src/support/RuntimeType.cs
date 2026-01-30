using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.RuntimeType")]
internal class RuntimeType : Type
{
    [UsedImplicitly] private nint index;
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type MakeGenericType(params Type[] typeArguments);

    public override bool IsAssignableFrom(Type? c)
    {
        // https://learn.microsoft.com/en-us/dotnet/api/system.type.isassignablefrom?view=net-9.0#returns
        if (c == null)
            return false;

        if (c == this)
            return true;

        // Check inheritance
        var baseType = c.BaseType;
        while (baseType != null)
        {
            if (baseType == this)
                return true;
            baseType = baseType.BaseType;
        }

        // Check interface implementation
        if (IsInterface)
        {
            var interfaces = c.GetInterfaces();
            if (interfaces.Any(t => t == this))
            {
                return true;
            }
        }

        // Check generic parameter constraints
        if (c.IsGenericParameter)
        {
            var constraints = c.GetGenericParameterConstraints();
            if (constraints.Any(t => t == this))
            {
                return true;
            }
        }

        // Check nullable value types
        if (c.IsValueType && IsGenericType && GetGenericTypeDefinition() == typeof(Nullable<>))
        {
            return GetGenericArguments()[0] == c;
        }

        return false;
    }

    public new bool IsAssignableTo(Type? targetType)
    {
        // IsAssignableTo is the inverse of IsAssignableFrom
        // this.IsAssignableTo(target) means target.IsAssignableFrom(this)
        return targetType?.IsAssignableFrom(this) ?? false;
    }
    
    public override extern System.Reflection.Assembly Assembly { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string? GetQualifiedName();
    public override string? AssemblyQualifiedName => GetQualifiedName();

    public override extern Type? BaseType { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public override extern string Name { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public override string FullName
    {
        get
        {
            var name = Name;
            var ns = Namespace;
            return ns == null ? name : $"{ns}.{name}";
        }
    }

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Module GetModule();
    public override System.Reflection.Module Module => GetModule();

    public override extern string? Namespace { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public override extern bool IsGenericType { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type GetGenericTypeDefinition();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type[] GetGenericArguments();

    [MethodImpl(MethodImplOptions.InternalCall)]
    internal extern object CreateInstanceDefaultCtor(bool publicOnly, bool skipCheck);

    public override extern System.RuntimeTypeHandle TypeHandle { [MethodImpl(MethodImplOptions.InternalCall)] get; }

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
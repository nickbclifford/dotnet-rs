using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.RuntimeType")]
internal class RuntimeType : Type
{
    [RuntimeSlot(RuntimeSlotId.RuntimeTypeIndex)]
    [UsedImplicitly] private nint index;
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type MakeGenericType(params Type[] typeArguments);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type MakeByRefType();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type MakeArrayType();

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

    public override extern string? FullName { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern string ToString();

    public override System.Reflection.Module Module => DotnetRs.Module.ForAssembly(Assembly);

    public override extern string? Namespace { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public override extern bool IsGenericType { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public override extern bool IsGenericTypeDefinition { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public override extern bool IsGenericParameter { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type GetGenericTypeDefinition();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern Type[] GetGenericArguments();

    [MethodImpl(MethodImplOptions.InternalCall)]
    internal extern object CreateInstanceDefaultCtor(bool publicOnly, bool skipCheck);

    // Replaces CoreLib's `RuntimeType.IsActualInterface`, which reads the method table through
    // `GetNativeTypeHandle` (`m_handle`). `DynamicMethod.Init` checks it for an owner type; that
    // read raised AccessViolationException before this override existed (plan 24 slice 2).
    internal bool IsActualInterface => IsInterface;

    // Replaces CoreLib's `RuntimeType.CreateInstanceImpl`, the funnel for every
    // `Activator.CreateInstance(Type, BindingFlags, Binder, object[], CultureInfo, ...)` overload.
    // CoreLib's body enumerates constructors through `RuntimeType.Cache`, which reads `m_cache`, a
    // field this stub doesn't have. This mirrors that body with `GetConstructors` and the same
    // candidate filter (`RuntimeType.FilterApplyMethodBase`) instead.
    internal object? CreateInstanceImpl(BindingFlags bindingAttr, Binder? binder, object?[]? args, CultureInfo? culture)
    {
        if (ContainsGenericParameters)
            throw new ArgumentException(
                $"Cannot create an instance of {this} because Type.ContainsGenericParameters is true.");
        args ??= [];
        binder ??= DefaultBinder;
        var publicOnly = (bindingAttr & BindingFlags.NonPublic) == 0;
        var wrapExceptions = (bindingAttr & BindingFlags.DoNotWrapExceptions) == 0;

        if (args.Length == 0 && (bindingAttr & BindingFlags.Public) != 0 &&
            (bindingAttr & BindingFlags.Instance) != 0 && IsValueType)
            return CreateInstanceDefaultCtor(publicOnly, wrapExceptions);

        var argumentTypes = args.Length == 0 ? EmptyTypes : new Type?[args.Length];
        for (var i = 0; i < args.Length; i++)
            argumentTypes[i] = args[i]?.GetType();

        var all = GetConstructors(bindingAttr);
        var candidates = new MethodBase[all.Length];
        var count = 0;
        foreach (var constructor in all)
        {
            if (ConstructorAcceptsArity(constructor, bindingAttr, argumentTypes))
                candidates[count++] = constructor;
        }
        if (count == 0)
            throw new MissingMethodException($"Constructor on type '{FullName}' not found.");
        if (count != candidates.Length)
            Array.Resize(ref candidates, count);

        MethodBase? selected;
        object? state;
        try
        {
            selected = binder.BindToMethod(bindingAttr, candidates, ref args, null, culture, null, out state);
        }
        catch (MissingMethodException inner)
        {
            throw new MissingMethodException($"Constructor on type '{FullName}' not found.", inner);
        }
        if (selected is null)
            throw new MissingMethodException($"Constructor on type '{FullName}' not found.");

        if (selected.GetParameters().Length == 0)
        {
            if (args.Length != 0)
                throw new NotSupportedException("Vararg calling convention not supported.");
            return CreateInstanceDefaultCtor(publicOnly: false, wrapExceptions);
        }

        var result = ((ConstructorInfo)selected).Invoke(bindingAttr, binder, args, culture);
        if (state != null)
            binder.ReorderArgumentArray(ref args, state);
        return result;
    }

    // The argument-count part of CoreLib's `RuntimeType.FilterApplyMethodBase`; `GetConstructors`
    // has already applied the binding-flag part.
    private static bool ConstructorAcceptsArity(ConstructorInfo constructor, BindingFlags bindingAttr, Type?[] argumentTypes)
    {
        var parameters = constructor.GetParameters();
        if (argumentTypes.Length == parameters.Length)
        {
            if ((bindingAttr & BindingFlags.ExactBinding) == 0 || (bindingAttr & BindingFlags.InvokeMethod) != 0)
                return true;
            for (var i = 0; i < parameters.Length; i++)
            {
                var type = argumentTypes[i];
                if (type is not null && type != parameters[i].ParameterType)
                    return false;
            }
            return true;
        }

        const BindingFlags invokeFlags = BindingFlags.InvokeMethod | BindingFlags.CreateInstance |
                                         BindingFlags.GetProperty | BindingFlags.SetProperty;
        if ((bindingAttr & invokeFlags) == 0)
            return false;

        bool needsParamArray;
        if (argumentTypes.Length > parameters.Length)
            needsParamArray = (constructor.CallingConvention & CallingConventions.VarArgs) == 0;
        else if ((bindingAttr & BindingFlags.OptionalParamBinding) == 0)
            needsParamArray = true;
        else
            needsParamArray = !parameters[argumentTypes.Length].IsOptional;
        if (!needsParamArray)
            return true;

        if (parameters.Length == 0 || argumentTypes.Length < parameters.Length - 1)
            return false;
        var last = parameters[^1];
        return last.ParameterType.IsArray && last.IsDefined(typeof(ParamArrayAttribute), false);
    }

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
    public override extern MemberInfo[] GetMember(string name, BindingFlags bindingAttr);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern MemberInfo[] GetMember(string name, MemberTypes type, BindingFlags bindingAttr);

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
    protected override extern System.Reflection.MethodInfo? GetMethodImpl(
        string name,
        int genericParameterCount,
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

    public override IList<CustomAttributeData> GetCustomAttributesData() =>
        new List<CustomAttributeData>();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern bool IsDefined(Type attributeType, bool inherit);

    // The base `Type.GetTypeCodeImpl` returns `TypeCode.Object` for everything because
    // `UnderlyingSystemType` is `this`. The real runtime `RuntimeType` overrides this to
    // map the intrinsic primitives onto their `TypeCode`; without it `Type.GetTypeCode`
    // reports `Object` for `int`/`double`/etc., which breaks consumers such as
    // `System.Linq.Expressions` (`TypeUtils.IsArithmetic`) that switch on the type code.
    protected override TypeCode GetTypeCodeImpl()
    {
        if (IsEnum)
        {
            return GetTypeCode(GetEnumUnderlyingType());
        }

        if (this == typeof(bool)) return TypeCode.Boolean;
        if (this == typeof(char)) return TypeCode.Char;
        if (this == typeof(sbyte)) return TypeCode.SByte;
        if (this == typeof(byte)) return TypeCode.Byte;
        if (this == typeof(short)) return TypeCode.Int16;
        if (this == typeof(ushort)) return TypeCode.UInt16;
        if (this == typeof(int)) return TypeCode.Int32;
        if (this == typeof(uint)) return TypeCode.UInt32;
        if (this == typeof(long)) return TypeCode.Int64;
        if (this == typeof(ulong)) return TypeCode.UInt64;
        if (this == typeof(float)) return TypeCode.Single;
        if (this == typeof(double)) return TypeCode.Double;
        if (this == typeof(decimal)) return TypeCode.Decimal;
        if (this == typeof(DateTime)) return TypeCode.DateTime;
        if (this == typeof(string)) return TypeCode.String;
        if (this == typeof(DBNull)) return TypeCode.DBNull;
        return TypeCode.Object;
    }
}

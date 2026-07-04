using System.Globalization;
using JetBrains.Annotations;

namespace DotnetRs;

public class PropertyInfo : System.Reflection.PropertyInfo
{
    [UsedImplicitly] private string? name;
    [UsedImplicitly] private System.Reflection.MethodInfo? getter;
    [UsedImplicitly] private System.Reflection.MethodInfo? setter;
    [UsedImplicitly] private Type? declaringType;
    [UsedImplicitly] private Type? propertyType;

    public override System.Reflection.PropertyAttributes Attributes =>
        System.Reflection.PropertyAttributes.None;

    public override bool CanRead => getter != null;
    public override bool CanWrite => setter != null;
    public override Type PropertyType => propertyType ?? typeof(object);
    public override Type? DeclaringType => declaringType;
    public override Type? ReflectedType => declaringType;
    public override string Name => name ?? string.Empty;

    public override System.Reflection.MethodInfo[] GetAccessors(bool nonPublic)
    {
        if (getter == null && setter == null)
            return System.Array.Empty<System.Reflection.MethodInfo>();

        if (nonPublic)
        {
            if (getter == null)
                return new System.Reflection.MethodInfo[] { setter! };
            if (setter == null)
                return new System.Reflection.MethodInfo[] { getter };
            return new System.Reflection.MethodInfo[] { getter, setter };
        }

        var count = 0;
        if (getter is { IsPublic: true }) count++;
        if (setter is { IsPublic: true }) count++;

        var accessors = new System.Reflection.MethodInfo[count];
        var index = 0;
        if (getter is { IsPublic: true }) accessors[index++] = getter;
        if (setter is { IsPublic: true }) accessors[index] = setter;
        return accessors;
    }

    public override System.Reflection.MethodInfo? GetGetMethod(bool nonPublic)
    {
        if (getter == null)
            return null;

        if (!nonPublic && !getter.IsPublic)
            return null;

        return getter;
    }

    public override System.Reflection.MethodInfo? GetSetMethod(bool nonPublic)
    {
        if (setter == null)
            return null;

        if (!nonPublic && !setter.IsPublic)
            return null;

        return setter;
    }

    public override System.Reflection.ParameterInfo[] GetIndexParameters()
    {
        if (getter != null)
            return getter.GetParameters();

        if (setter == null)
            return System.Array.Empty<System.Reflection.ParameterInfo>();

        var parameters = setter.GetParameters();
        if (parameters.Length <= 1)
            return System.Array.Empty<System.Reflection.ParameterInfo>();

        var indexParameters = new System.Reflection.ParameterInfo[parameters.Length - 1];
        System.Array.Copy(parameters, indexParameters, parameters.Length - 1);
        return indexParameters;
    }

    public override object[] GetCustomAttributes(bool inherit) => System.Array.Empty<object>();

    public override object[] GetCustomAttributes(Type attributeType, bool inherit) =>
        System.Array.Empty<object>();

    public override System.Collections.Generic.IList<System.Reflection.CustomAttributeData> GetCustomAttributesData() =>
        new System.Collections.Generic.List<System.Reflection.CustomAttributeData>();

    public override bool IsDefined(Type attributeType, bool inherit) => false;

    public override object? GetValue(
        object? obj,
        System.Reflection.BindingFlags invokeAttr,
        System.Reflection.Binder? binder,
        object?[]? index,
        CultureInfo? culture
    )
    {
        if (getter == null)
            throw new ArgumentException("Property get method was not found.");

        return getter.Invoke(obj, invokeAttr, binder, index, culture);
    }

    public override void SetValue(
        object? obj,
        object? value,
        System.Reflection.BindingFlags invokeAttr,
        System.Reflection.Binder? binder,
        object?[]? index,
        CultureInfo? culture
    )
    {
        if (setter == null)
            throw new ArgumentException("Property set method was not found.");

        var indexArguments = index ?? System.Array.Empty<object?>();
        var invokeArguments = new object?[indexArguments.Length + 1];
        System.Array.Copy(indexArguments, invokeArguments, indexArguments.Length);
        invokeArguments[^1] = value;
        setter.Invoke(obj, invokeAttr, binder, invokeArguments, culture);
    }
}

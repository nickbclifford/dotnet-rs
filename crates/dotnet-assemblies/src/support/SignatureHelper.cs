using System.Reflection;
using System.Reflection.Emit;

namespace DotnetRs;

// Record method signatures as types, avoiding CoreLib's native RuntimeTypeHandle encoding.
[Stub(InPlaceOf = "System.Reflection.Emit.SignatureHelper")]
public sealed class RecordedSignatureHelper
{
    private enum SignatureKind { Method, Local, Field, Property }
    private readonly SignatureKind _kind;
    private readonly CallingConventions _callingConvention;
    private readonly Type _returnType;
    private readonly List<Type> _arguments = new();
    private readonly List<bool> _pinned = new();
    private readonly List<int> _sentinels = new();
    private byte[]? _completed;
    private bool _isComplete;

    private RecordedSignatureHelper(CallingConventions callingConvention, Type? returnType,
        SignatureKind kind = SignatureKind.Method)
    {
        _kind = kind;
        _callingConvention = callingConvention;
        _returnType = returnType ?? typeof(void);
    }

    private static System.Reflection.Emit.SignatureHelper AsPublic(RecordedSignatureHelper helper) =>
        (System.Reflection.Emit.SignatureHelper)(object)helper;

    public static System.Reflection.Emit.SignatureHelper GetMethodSigHelper(CallingConventions callingConvention, Type? returnType) =>
        AsPublic(new RecordedSignatureHelper(callingConvention, returnType));

    public static System.Reflection.Emit.SignatureHelper GetMethodSigHelper(System.Reflection.Module? module, CallingConventions callingConvention, Type? returnType) =>
        AsPublic(new RecordedSignatureHelper(callingConvention, returnType));

    public static System.Reflection.Emit.SignatureHelper GetMethodSigHelper(System.Reflection.Module? module, Type? returnType, Type[]? parameterTypes)
    {
        var helper = new RecordedSignatureHelper(CallingConventions.Standard, returnType);
        if (parameterTypes != null) foreach (Type type in parameterTypes) helper.AddArgument(type);
        return AsPublic(helper);
    }

    public static System.Reflection.Emit.SignatureHelper GetLocalVarSigHelper() =>
        AsPublic(new RecordedSignatureHelper(CallingConventions.Standard, null, SignatureKind.Local));

    public static System.Reflection.Emit.SignatureHelper GetLocalVarSigHelper(System.Reflection.Module? module) =>
        GetLocalVarSigHelper();

    public static System.Reflection.Emit.SignatureHelper GetFieldSigHelper(System.Reflection.Module? module) =>
        AsPublic(new RecordedSignatureHelper(CallingConventions.Standard, null, SignatureKind.Field));

    public static System.Reflection.Emit.SignatureHelper GetPropertySigHelper(System.Reflection.Module? module,
        Type? returnType, Type[]? parameterTypes) =>
        GetPropertySigHelper(module, CallingConventions.Standard, returnType, null, null, parameterTypes, null, null);

    public static System.Reflection.Emit.SignatureHelper GetPropertySigHelper(System.Reflection.Module? module,
        Type? returnType, Type[]? requiredReturnTypeCustomModifiers, Type[]? optionalReturnTypeCustomModifiers,
        Type[]? parameterTypes, Type[][]? requiredParameterTypeCustomModifiers, Type[][]? optionalParameterTypeCustomModifiers) =>
        GetPropertySigHelper(module, CallingConventions.Standard, returnType, requiredReturnTypeCustomModifiers,
            optionalReturnTypeCustomModifiers, parameterTypes, requiredParameterTypeCustomModifiers, optionalParameterTypeCustomModifiers);

    public static System.Reflection.Emit.SignatureHelper GetPropertySigHelper(System.Reflection.Module? module,
        CallingConventions callingConvention, Type? returnType, Type[]? requiredReturnTypeCustomModifiers,
        Type[]? optionalReturnTypeCustomModifiers, Type[]? parameterTypes,
        Type[][]? requiredParameterTypeCustomModifiers, Type[][]? optionalParameterTypeCustomModifiers)
    {
        if ((requiredReturnTypeCustomModifiers?.Length ?? 0) != 0 || (optionalReturnTypeCustomModifiers?.Length ?? 0) != 0)
            throw new NullReferenceException();
        var helper = new RecordedSignatureHelper(callingConvention, returnType, SignatureKind.Property);
        helper.AddArguments(parameterTypes, requiredParameterTypeCustomModifiers, optionalParameterTypeCustomModifiers);
        return AsPublic(helper);
    }

    public void AddArgument(Type argument)
    {
        ArgumentNullException.ThrowIfNull(argument);
        CheckMutable();
        _arguments.Add(argument);
        _pinned.Add(false);
    }

    public void AddArgument(Type argument, bool pinned)
    {
        AddArgument(argument);
        _pinned[_pinned.Count - 1] = pinned;
    }

    public void AddArgument(Type argument, Type[]? requiredCustomModifiers, Type[]? optionalCustomModifiers)
    {
        if ((requiredCustomModifiers?.Length ?? 0) != 0 || (optionalCustomModifiers?.Length ?? 0) != 0)
            throw new NullReferenceException();
        AddArgument(argument);
    }

    public void AddArguments(Type[]? arguments, Type[][]? requiredCustomModifiers, Type[][]? optionalCustomModifiers)
    {
        if (arguments == null) return;
        if ((requiredCustomModifiers != null && requiredCustomModifiers.Length != arguments.Length) ||
            (optionalCustomModifiers != null && optionalCustomModifiers.Length != arguments.Length))
            throw new ArgumentException("Custom modifier arrays must match the argument count.");
        for (int i = 0; i < arguments.Length; i++)
            AddArgument(arguments[i], requiredCustomModifiers?[i], optionalCustomModifiers?[i]);
    }

    public void AddSentinel()
    {
        CheckMutable();
        _sentinels.Add(_arguments.Count);
    }

    private void CheckMutable()
    {
        if (_isComplete) throw new ArgumentException("Completed signature cannot be modified.");
    }

    private static void AddCompressed(List<byte> bytes, int value)
    {
        if (value <= 0x7f) bytes.Add((byte)value);
        else if (value <= 0x3fff) { bytes.Add((byte)((value >> 8) | 0x80)); bytes.Add((byte)value); }
        else if (value <= 0x1fffffff)
        {
            bytes.Add((byte)((value >> 24) | 0xc0)); bytes.Add((byte)(value >> 16));
            bytes.Add((byte)(value >> 8)); bytes.Add((byte)value);
        }
        else throw new ArgumentOutOfRangeException(nameof(value));
    }

    private static void AddType(List<byte> bytes, Type type)
    {
        if (type == typeof(void)) bytes.Add(0x01);
        else if (type == typeof(bool)) bytes.Add(0x02);
        else if (type == typeof(char)) bytes.Add(0x03);
        else if (type == typeof(sbyte)) bytes.Add(0x04);
        else if (type == typeof(byte)) bytes.Add(0x05);
        else if (type == typeof(short)) bytes.Add(0x06);
        else if (type == typeof(ushort)) bytes.Add(0x07);
        else if (type == typeof(int)) bytes.Add(0x08);
        else if (type == typeof(uint)) bytes.Add(0x09);
        else if (type == typeof(long)) bytes.Add(0x0a);
        else if (type == typeof(ulong)) bytes.Add(0x0b);
        else if (type == typeof(float)) bytes.Add(0x0c);
        else if (type == typeof(double)) bytes.Add(0x0d);
        else if (type == typeof(string)) bytes.Add(0x0e);
        else if (type == typeof(IntPtr)) bytes.Add(0x18);
        else if (type == typeof(UIntPtr)) bytes.Add(0x19);
        else if (type == typeof(object)) bytes.Add(0x1c);
        else if (type.IsByRef) { bytes.Add(0x10); AddType(bytes, type.GetElementType()!); }
        else if (type.IsPointer) { bytes.Add(0x0f); AddType(bytes, type.GetElementType()!); }
        else if (type.IsArray && type.GetElementType()!.MakeArrayType() == type)
        { bytes.Add(0x1d); AddType(bytes, type.GetElementType()!); }
        else throw new NotSupportedException("Raw signature encoding for this type is not supported yet.");
    }

    public byte[] GetSignature()
    {
        if (_completed != null) return (byte[])_completed.Clone();
        var bytes = new List<byte>();
        int convention = _kind switch
        {
            SignatureKind.Local => 7,
            SignatureKind.Field => 6,
            SignatureKind.Property => 8,
            _ => (_callingConvention & CallingConventions.VarArgs) != 0 ? 5 : 0,
        };
        if ((_callingConvention & CallingConventions.HasThis) != 0) convention |= 0x20;
        if ((_callingConvention & CallingConventions.ExplicitThis) != 0) convention |= 0x40;
        bytes.Add((byte)convention);
        if (_kind != SignatureKind.Field) AddCompressed(bytes, _arguments.Count);
        if (_kind == SignatureKind.Method || _kind == SignatureKind.Property) AddType(bytes, _returnType);
        int sentinel = 0;
        for (int i = 0; i <= _arguments.Count; i++)
        {
            while (sentinel < _sentinels.Count && _sentinels[sentinel] == i)
            {
                bytes.Add(0x41);
                sentinel++;
            }
            if (i == _arguments.Count) break;
            if (_pinned[i]) bytes.Add(0x45);
            AddType(bytes, _arguments[i]);
        }
        _completed = bytes.ToArray();
        _isComplete = true;
        return (byte[])_completed.Clone();
    }

    internal void EmitTo(RecordingILGenerator generator, OpCode opcode)
    {
        if (_kind != SignatureKind.Method) throw new ArgumentException("A method signature is required.");
        // The CLR accepts local-only pinned markers in method signatures without pinning the
        // argument, and ignores sentinels in standard managed call-site signatures. Preserve
        // their bytes in GetSignature, but normalize them for the resolved call-site signature.
        int fixedCount = (_callingConvention & CallingConventions.VarArgs) != 0 && _sentinels.Count != 0
            ? _sentinels[0] : _arguments.Count;
        generator.RecordCalli(opcode, (int)_callingConvention, _returnType, _arguments.ToArray(), fixedCount);
        _isComplete = true;
    }
}

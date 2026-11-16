using System.Globalization;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;

namespace DotnetRs;

// Plan 24 slice 4 SPIKE. Replaces CoreLib's DynamicMethod with a recording shim: the IL generator
// records resolved operations (opcode id, immediate, reference operand) instead of raw IL bytes
// and DynamicScope tokens, and the VM turns the record into a one-method in-memory assembly on
// first use. CoreLib's DynamicILGenerator/DynamicScope/DynamicResolver are bypassed entirely.
[Stub(InPlaceOf = "System.Reflection.Emit.DynamicMethod")]
public sealed class DynamicMethod : System.Reflection.MethodInfo
{
    private readonly string _name;
    private readonly Type _returnType;
    private readonly Type[] _parameterTypes;
    private RecordingILGenerator? _il;
    private System.Reflection.MethodInfo? _built;

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes)
    {
        ArgumentNullException.ThrowIfNull(name);
        _name = name;
        _returnType = returnType ?? typeof(void);
        _parameterTypes = parameterTypes ?? Type.EmptyTypes;
    }

    // Spike shortcut: owner/module association and skipVisibility are validated or ignored, not
    // modelled. The CLR places the method in the owner's module so it can reach private members.
    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, bool restrictedSkipVisibility)
        : this(name, returnType, parameterTypes) { }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, System.Reflection.Module m)
        : this(name, returnType, parameterTypes)
    {
        ArgumentNullException.ThrowIfNull(m);
    }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, System.Reflection.Module m,
        bool skipVisibility) : this(name, returnType, parameterTypes, m) { }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, Type owner)
        : this(name, returnType, parameterTypes)
    {
        ValidateOwner(owner);
    }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, Type owner, bool skipVisibility)
        : this(name, returnType, parameterTypes, owner) { }

    public DynamicMethod(string name, MethodAttributes attributes, CallingConventions callingConvention,
        Type? returnType, Type[]? parameterTypes, Type owner, bool skipVisibility)
        : this(name, returnType, parameterTypes, owner) { }

    public DynamicMethod(string name, MethodAttributes attributes, CallingConventions callingConvention,
        Type? returnType, Type[]? parameterTypes, System.Reflection.Module m, bool skipVisibility)
        : this(name, returnType, parameterTypes, m) { }

    private static void ValidateOwner(Type owner)
    {
        ArgumentNullException.ThrowIfNull(owner);
        if (owner.IsArray || owner.IsInterface || owner.ContainsGenericParameters)
            throw new ArgumentException("Invalid type for a dynamic method owner.");
    }

    public ILGenerator GetILGenerator() => _il ??= new RecordingILGenerator();

    private System.Reflection.MethodInfo Built()
    {
        if (_built != null) return _built;
        if (_il == null)
            throw new InvalidOperationException("Method body is empty.");
        _built = _il.Compile(_name, _returnType, _parameterTypes);
        return _built;
    }

    public override System.Delegate CreateDelegate(Type delegateType) => Built().CreateDelegate(delegateType);

    public override System.Delegate CreateDelegate(Type delegateType, object? target) =>
        Built().CreateDelegate(delegateType, target);

    public override object? Invoke(object? obj, BindingFlags invokeAttr, Binder? binder,
        object?[]? parameters, CultureInfo? culture) =>
        Built().Invoke(obj, invokeAttr, binder, parameters, culture);

    public override string Name => _name;
    public override Type? DeclaringType => null;
    public override Type? ReflectedType => null;
    public override Type ReturnType => _returnType;
    public override MethodAttributes Attributes => MethodAttributes.Public | MethodAttributes.Static;
    public override CallingConventions CallingConvention => CallingConventions.Standard;
    public override System.RuntimeMethodHandle MethodHandle => throw new InvalidOperationException();
    public override ICustomAttributeProvider ReturnTypeCustomAttributes => throw new NotSupportedException();
    public override System.Reflection.MethodInfo GetBaseDefinition() => this;
    public override MethodImplAttributes GetMethodImplementationFlags() => MethodImplAttributes.IL;
    public override ParameterInfo[] GetParameters() => throw new NotSupportedException();
    public override object[] GetCustomAttributes(bool inherit) => System.Array.Empty<object>();
    public override object[] GetCustomAttributes(Type attributeType, bool inherit) => System.Array.Empty<object>();
    public override bool IsDefined(Type attributeType, bool inherit) => false;
}

internal sealed class RecordingLocal : LocalBuilder
{
    private readonly int _index;
    private readonly Type _type;
    public RecordingLocal(int index, Type type) { _index = index; _type = type; }
    public override int LocalIndex => _index;
    public override Type LocalType => _type;
    public override bool IsPinned => false;
}

internal sealed class RecordingILGenerator : ILGenerator
{
    // One entry per instruction. Reference operands are Type/MethodBase/string.
    private readonly List<int> _ops = new();
    private readonly List<long> _imm = new();
    private readonly List<object?> _refs = new();
    private readonly List<Type> _locals = new();
    // Five ints per clause: kind (0 catch, 2 finally), try start, try end, handler start, handler end.
    private readonly List<int> _eh = new();
    private readonly List<Type?> _ehTypes = new();

    private sealed class Block
    {
        public int TryStart, TryEnd, HandlerStart, Kind = -1;
        public Type? CatchType;
        public readonly List<int> LeaveSites = new();
    }
    private readonly Stack<Block> _blocks = new();

    private void Record(OpCode op, long imm, object? reference)
    {
        _ops.Add(op.Value);
        _imm.Add(imm);
        _refs.Add(reference);
    }

    public override int ILOffset => _ops.Count;

    public override void Emit(OpCode opcode) => Record(opcode, 0, null);
    public override void Emit(OpCode opcode, byte arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, short arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, int arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, long arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, float arg) => Record(opcode, BitConverter.SingleToInt32Bits(arg), null);
    public override void Emit(OpCode opcode, double arg) => Record(opcode, BitConverter.DoubleToInt64Bits(arg), null);
    public override void Emit(OpCode opcode, string str) => Record(opcode, 0, str);
    public override void Emit(OpCode opcode, Type? cls) => Record(opcode, 0, cls);
    public override void Emit(OpCode opcode, System.Reflection.MethodInfo meth) => Record(opcode, 0, meth);
    public override void Emit(OpCode opcode, System.Reflection.ConstructorInfo con) => Record(opcode, 0, con);
    public override void Emit(OpCode opcode, LocalBuilder local) => Record(opcode, local.LocalIndex, null);
    public override void Emit(OpCode opcode, System.Reflection.FieldInfo field) => throw new NotSupportedException("spike: field operands");
    public override void Emit(OpCode opcode, Label label) => throw new NotSupportedException("spike: labels");
    public override void Emit(OpCode opcode, Label[] labels) => throw new NotSupportedException("spike: labels");
    public override void Emit(OpCode opcode, SignatureHelper signature) => throw new NotSupportedException("spike: signatures");
    public override void EmitCall(OpCode opcode, System.Reflection.MethodInfo methodInfo, Type[]? optionalParameterTypes) =>
        Record(opcode, 0, methodInfo);
    public override void EmitCalli(OpCode opcode, CallingConventions callingConvention, Type? returnType,
        Type[]? parameterTypes, Type[]? optionalParameterTypes) => throw new NotSupportedException("spike: calli");
    public override void EmitCalli(OpCode opcode, System.Runtime.InteropServices.CallingConvention unmanagedCallConv,
        Type? returnType, Type[]? parameterTypes) => throw new NotSupportedException("spike: calli");

    public override LocalBuilder DeclareLocal(Type localType, bool pinned)
    {
        _locals.Add(localType);
        return new RecordingLocal(_locals.Count - 1, localType);
    }

    public override Label DefineLabel() => throw new NotSupportedException("spike: labels");
    public override void MarkLabel(Label loc) => throw new NotSupportedException("spike: labels");

    public override Label BeginExceptionBlock()
    {
        _blocks.Push(new Block { TryStart = _ops.Count });
        return default;
    }

    private void CloseHandler(Block b, int kind)
    {
        // The CLR's ILGenerator ends each protected/handler region with a leave to the block end.
        if (b.Kind < 0)
        {
            b.LeaveSites.Add(_ops.Count);
            Record(OpCodes.Leave, 0, null);
            b.TryEnd = _ops.Count;
        }
        else
        {
            CloseCurrentHandler(b);
        }
        b.Kind = kind;
        b.HandlerStart = _ops.Count;
    }

    private void CloseCurrentHandler(Block b)
    {
        if (b.Kind == 2)
        {
            Record(OpCodes.Endfinally, 0, null);
        }
        else
        {
            b.LeaveSites.Add(_ops.Count);
            Record(OpCodes.Leave, 0, null);
        }
        _eh.Add(b.Kind); _eh.Add(b.TryStart); _eh.Add(b.TryEnd); _eh.Add(b.HandlerStart); _eh.Add(_ops.Count);
        _ehTypes.Add(b.CatchType);
    }

    public override void BeginCatchBlock(Type? exceptionType)
    {
        var b = _blocks.Peek();
        CloseHandler(b, 0);
        b.CatchType = exceptionType ?? typeof(object);
    }

    public override void BeginFinallyBlock()
    {
        var b = _blocks.Peek();
        CloseHandler(b, 2);
        b.CatchType = null;
    }

    public override void EndExceptionBlock()
    {
        var b = _blocks.Pop();
        CloseCurrentHandler(b);
        int end = _ops.Count;
        foreach (var site in b.LeaveSites) _imm[site] = end;
    }

    public override void BeginExceptFilterBlock() => throw new NotSupportedException("spike: filters");
    public override void BeginFaultBlock() => throw new NotSupportedException("spike: fault");
    public override void BeginScope() { }
    public override void EndScope() { }
    public override void UsingNamespace(string usingNamespace) { }

    [MethodImpl(MethodImplOptions.InternalCall)]
    private static extern System.Reflection.MethodInfo CompileRecorded(string name, Type returnType,
        Type[] parameterTypes, int[] ops, long[] imm, object?[] refs, Type[] locals, int[] eh, Type?[] ehTypes);

    public System.Reflection.MethodInfo Compile(string name, Type returnType, Type[] parameterTypes)
    {
        if (_blocks.Count != 0)
            throw new InvalidOperationException("Unclosed exception block.");
        return CompileRecorded(name, returnType, parameterTypes, _ops.ToArray(), _imm.ToArray(),
            _refs.ToArray(), _locals.ToArray(), _eh.ToArray(), _ehTypes.ToArray());
    }
}

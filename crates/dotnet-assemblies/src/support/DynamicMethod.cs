using System.Globalization;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;

namespace DotnetRs;

// Plan 24, completed by plan 25. Replaces CoreLib's DynamicMethod with a recording shim: the IL
// generator records resolved operations (opcode id, immediate, reference operand) instead of raw
// IL bytes and DynamicScope tokens, and the VM turns the record into a one-method in-memory
// assembly on first use. CoreLib's DynamicILGenerator/DynamicScope/DynamicResolver are bypassed.
[Stub(InPlaceOf = "System.Reflection.Emit.DynamicMethod")]
public sealed class DynamicMethod : System.Reflection.MethodInfo
{
    private readonly string _name;
    private readonly Type _returnType;
    private readonly Type[] _parameterTypes;
    private System.Reflection.Module _module = DotnetRs.Module.Anonymous;
    private DynamicParameterInfo[]? _parameters;
    private RecordingILGenerator? _il;
    private System.Reflection.MethodInfo? _built;

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes)
    {
        ArgumentNullException.ThrowIfNull(name);
        _name = name;
        _returnType = returnType ?? typeof(void);
        _parameterTypes = parameterTypes == null ? Type.EmptyTypes : (Type[])parameterTypes.Clone();
        foreach (var type in _parameterTypes)
            if (type == null || type == typeof(void))
                throw new ArgumentException("Invalid type in signature.");
    }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, bool restrictedSkipVisibility)
        : this(name, returnType, parameterTypes) { }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, System.Reflection.Module m)
        : this(name, returnType, parameterTypes)
    {
        ArgumentNullException.ThrowIfNull(m);
        _module = m;
    }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, System.Reflection.Module m,
        bool skipVisibility) : this(name, returnType, parameterTypes, m) { }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, Type owner)
        : this(name, returnType, parameterTypes)
    {
        ValidateOwner(owner);
        _module = owner.Module;
    }

    public DynamicMethod(string name, Type? returnType, Type[]? parameterTypes, Type owner, bool skipVisibility)
        : this(name, returnType, parameterTypes, owner) { }

    public DynamicMethod(string name, MethodAttributes attributes, CallingConventions callingConvention,
        Type? returnType, Type[]? parameterTypes, Type owner, bool skipVisibility)
        : this(name, returnType, parameterTypes, owner) => ValidateFlags(attributes, callingConvention);

    public DynamicMethod(string name, MethodAttributes attributes, CallingConventions callingConvention,
        Type? returnType, Type[]? parameterTypes, System.Reflection.Module m, bool skipVisibility)
        : this(name, returnType, parameterTypes, m) => ValidateFlags(attributes, callingConvention);

    private static void ValidateFlags(MethodAttributes attributes, CallingConventions callingConvention)
    {
        if (attributes != (MethodAttributes.Public | MethodAttributes.Static) ||
            callingConvention != CallingConventions.Standard)
            throw new NotSupportedException("Invalid dynamic method flags.");
    }

    private static void ValidateOwner(Type owner)
    {
        ArgumentNullException.ThrowIfNull(owner);
        if (owner.HasElementType || owner.IsInterface || owner.IsGenericParameter || owner.ContainsGenericParameters)
            throw new ArgumentException("Invalid type for a dynamic method owner.");
    }

    public ILGenerator GetILGenerator() => _il ??= new RecordingILGenerator();
    public ILGenerator GetILGenerator(int streamSize) => GetILGenerator();
    public DynamicILInfo GetDynamicILInfo() => throw new NotSupportedException("DynamicMethod.GetDynamicILInfo is not supported.");

    public ParameterBuilder? DefineParameter(int position, ParameterAttributes attributes, string? parameterName)
    {
        if (position < 0 || position > _parameterTypes.Length)
            throw new ArgumentOutOfRangeException(nameof(position));
        if (position > 0)
            Parameters()[position - 1].Define(attributes, parameterName);
        return null;
    }

    private DynamicParameterInfo[] Parameters()
    {
        if (_parameters != null) return _parameters;
        var parameters = new DynamicParameterInfo[_parameterTypes.Length];
        for (int i = 0; i < parameters.Length; i++)
            parameters[i] = new DynamicParameterInfo(this, _parameterTypes[i], i);
        return _parameters = parameters;
    }

    private sealed class DynamicParameterInfo : System.Reflection.ParameterInfo
    {
        internal DynamicParameterInfo(DynamicMethod method, Type type, int position)
        {
            MemberImpl = method;
            ClassImpl = type;
            PositionImpl = position;
        }

        public override bool HasDefaultValue => false;
        public override object? DefaultValue => null;
        public override object? RawDefaultValue => null;
        // CoreCLR 10 attempts to read metadata from a missing module for these parameters.
        public override IList<CustomAttributeData> GetCustomAttributesData() => throw new NullReferenceException();

        internal void Define(ParameterAttributes attributes, string? name)
        {
            AttrsImpl = attributes;
            NameImpl = name;
        }
    }

    private sealed class EmptyAttributes : ICustomAttributeProvider
    {
        public object[] GetCustomAttributes(bool inherit) => System.Array.Empty<object>();
        public object[] GetCustomAttributes(Type attributeType, bool inherit) => System.Array.Empty<object>();
        public bool IsDefined(Type attributeType, bool inherit) => false;
    }

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
    public override System.Reflection.Module Module => _module;
    public override Type? DeclaringType => null;
    public override Type? ReflectedType => null;
    public override Type ReturnType => _returnType;
    public override MethodAttributes Attributes => MethodAttributes.Public | MethodAttributes.Static;
    public override CallingConventions CallingConvention => CallingConventions.Standard;
    public override System.RuntimeMethodHandle MethodHandle => throw new InvalidOperationException();
    public override ICustomAttributeProvider ReturnTypeCustomAttributes => new EmptyAttributes();
    public override System.Reflection.ParameterInfo ReturnParameter => new DynamicParameterInfo(this, _returnType, -1);
    public override System.Reflection.MethodInfo GetBaseDefinition() => this;
    public override MethodImplAttributes GetMethodImplementationFlags() => MethodImplAttributes.IL;
    public override System.Reflection.ParameterInfo[] GetParameters()
    {
        var parameters = Parameters();
        var copy = new System.Reflection.ParameterInfo[parameters.Length];
        for (int i = 0; i < copy.Length; i++) copy[i] = parameters[i];
        return copy;
    }
    public override object[] GetCustomAttributes(bool inherit) => System.Array.Empty<object>();
    public override object[] GetCustomAttributes(Type attributeType, bool inherit) => System.Array.Empty<object>();
    public override bool IsDefined(Type attributeType, bool inherit) => false;
}

internal sealed class RecordingLocal : LocalBuilder
{
    private readonly int _index;
    private readonly Type _type;
    private readonly bool _pinned;
    public RecordingLocal(int index, Type type, bool pinned) { _index = index; _type = type; _pinned = pinned; }
    public override int LocalIndex => _index;
    public override Type LocalType => _type;
    public override bool IsPinned => _pinned;
}

internal sealed class RecordingILGenerator : ILGenerator
{
    // One entry per instruction. Reference operands include types, members, strings and targets.
    private readonly List<int> _ops = new();
    private readonly List<OpCode> _codes = new();
    private readonly List<int> _offsets = new();
    private int _ilOffset;
    private readonly List<long> _imm = new();
    private readonly List<object?> _refs = new();
    private readonly List<Type> _locals = new();
    private readonly List<int> _localPins = new();
    // Six ints per clause: kind, try start/end, handler start/end, filter start.
    private readonly List<int> _eh = new();
    private readonly List<Type?> _ehTypes = new();
    private readonly List<int> _labels = new();
    private readonly List<(int Site, int Label)> _branches = new();
    private readonly List<(int Site, int[] Labels)> _switches = new();

    private sealed class Block
    {
        public int TryStart, TryEnd, HandlerStart, FilterStart, Kind = -1;
        public Label EndLabel;
        public Type? CatchType;
        public readonly List<int> LeaveSites = new();
    }
    private readonly Stack<Block> _blocks = new();

    private Block CurrentBlock()
    {
        if (_blocks.Count == 0) throw new NotSupportedException("Not in an exception block.");
        return _blocks.Peek();
    }

    private void Record(OpCode op, long imm, object? reference)
    {
        _offsets.Add(_ilOffset);
        _ops.Add(op.Value);
        _codes.Add(op);
        _imm.Add(imm);
        _refs.Add(reference);
        _ilOffset += op.Size + (op.OperandType switch
        {
            OperandType.InlineNone => 0,
            OperandType.ShortInlineBrTarget or OperandType.ShortInlineI or OperandType.ShortInlineVar => 1,
            OperandType.InlineVar => 2,
            OperandType.InlineI8 or OperandType.InlineR => 8,
            OperandType.InlineSwitch => 4 + 4 * (((int[]?)reference)?.Length ?? 0),
            _ => 4,
        });
    }

    public override int ILOffset => _ilOffset;

    public override void Emit(OpCode opcode) => Record(opcode, 0, null);
    public override void Emit(OpCode opcode, byte arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, short arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, int arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, long arg) => Record(opcode, arg, null);
    public override void Emit(OpCode opcode, float arg) => Record(opcode, BitConverter.SingleToInt32Bits(arg), null);
    public override void Emit(OpCode opcode, double arg) => Record(opcode, BitConverter.DoubleToInt64Bits(arg), null);
    public override void Emit(OpCode opcode, string str) => Record(opcode, 0, str);
    public override void Emit(OpCode opcode, Type? cls) => Record(opcode, opcode.Value == OpCodes.Ldtoken.Value ? 1 : 0, cls);
    public override void Emit(OpCode opcode, System.Reflection.MethodInfo meth) => Record(opcode, opcode.Value == OpCodes.Ldtoken.Value ? 2 : 0, meth);
    public override void Emit(OpCode opcode, System.Reflection.ConstructorInfo con) => Record(opcode, opcode.Value == OpCodes.Ldtoken.Value ? 2 : 0, con);
    public override void Emit(OpCode opcode, LocalBuilder local) => Record(opcode, local.LocalIndex, null);
    public override void Emit(OpCode opcode, System.Reflection.FieldInfo field) => Record(opcode, opcode.Value == OpCodes.Ldtoken.Value ? 3 : 0, field);
    public override void Emit(OpCode opcode, Label label)
    {
        _branches.Add((_ops.Count, Unsafe.As<Label, int>(ref label)));
        Record(opcode, 0, null);
    }
    public override void Emit(OpCode opcode, Label[] labels)
    {
        ArgumentNullException.ThrowIfNull(labels);
        var ids = new int[labels.Length];
        for (int i = 0; i < ids.Length; i++) ids[i] = Unsafe.As<Label, int>(ref labels[i]);
        _switches.Add((_ops.Count, ids));
        Record(opcode, 0, ids);
    }
    public override void Emit(OpCode opcode, SignatureHelper signature)
    {
        ArgumentNullException.ThrowIfNull(signature);
        ((RecordedSignatureHelper)(object)signature).EmitTo(this, opcode);
    }
    public override void EmitCall(OpCode opcode, System.Reflection.MethodInfo methodInfo, Type[]? optionalParameterTypes)
    {
        ArgumentNullException.ThrowIfNull(methodInfo);
        if (optionalParameterTypes != null && (methodInfo.CallingConvention & CallingConventions.VarArgs) == 0)
            throw new InvalidOperationException("Calling convention must be VarArgs.");
        if (optionalParameterTypes == null)
            Record(opcode, 0, methodInfo);
        else
            Record(opcode, 1, new object[] { methodInfo, optionalParameterTypes });
    }
    public override void EmitCalli(OpCode opcode, CallingConventions callingConvention, Type? returnType,
        Type[]? parameterTypes, Type[]? optionalParameterTypes)
    {
        if (optionalParameterTypes != null && (callingConvention & CallingConventions.VarArgs) == 0)
            throw new InvalidOperationException("Calling convention must be VarArgs.");
        parameterTypes ??= Type.EmptyTypes;
        optionalParameterTypes ??= Type.EmptyTypes;
        var args = new Type[parameterTypes.Length + optionalParameterTypes.Length];
        System.Array.Copy(parameterTypes, 0, args, 0, parameterTypes.Length);
        System.Array.Copy(optionalParameterTypes, 0, args, parameterTypes.Length, optionalParameterTypes.Length);
        RecordCalli(opcode, (int)callingConvention, returnType, args, parameterTypes.Length);
    }
    public override void EmitCalli(OpCode opcode, System.Runtime.InteropServices.CallingConvention unmanagedCallConv,
        Type? returnType, Type[]? parameterTypes)
    {
        parameterTypes ??= Type.EmptyTypes;
        RecordCalli(opcode, 0x100 | (int)unmanagedCallConv, returnType, parameterTypes, parameterTypes.Length);
    }

    internal void RecordCalli(OpCode opcode, int callingConvention, Type? returnType, Type[] arguments, int fixedCount)
    {
        var types = new Type[arguments.Length + 1];
        types[0] = returnType ?? typeof(void);
        System.Array.Copy(arguments, 0, types, 1, arguments.Length);
        Record(opcode, (long)callingConvention | ((long)fixedCount << 32), types);
    }

    public override LocalBuilder DeclareLocal(Type localType, bool pinned)
    {
        _locals.Add(localType);
        _localPins.Add(pinned ? 1 : 0);
        return new RecordingLocal(_locals.Count - 1, localType, pinned);
    }

    public override Label DefineLabel()
    {
        int id = _labels.Count;
        _labels.Add(-1);
        return Unsafe.As<int, Label>(ref id);
    }
    public override void MarkLabel(Label loc)
    {
        int id = Unsafe.As<Label, int>(ref loc);
        if ((uint)id >= (uint)_labels.Count || _labels[id] != -1)
            throw new ArgumentException("Invalid or already marked label.");
        _labels[id] = _ops.Count;
    }

    public override Label BeginExceptionBlock()
    {
        var end = DefineLabel();
        _blocks.Push(new Block { TryStart = _ops.Count, EndLabel = end });
        return end;
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
            if (b.Kind == 1 && b.HandlerStart < 0)
                throw new InvalidOperationException("Filter has no handler.");
            CloseCurrentHandler(b);
            // A finally after catches protects the complete try/catch construct.
            if (kind == 2) b.TryEnd = _ops.Count;
        }
        b.Kind = kind;
        b.HandlerStart = _ops.Count;
    }

    private void CloseCurrentHandler(Block b)
    {
        if (b.Kind == 2 || b.Kind == 4)
        {
            Record(OpCodes.Endfinally, 0, null);
        }
        else
        {
            b.LeaveSites.Add(_ops.Count);
            Record(OpCodes.Leave, 0, null);
        }
        _eh.Add(b.Kind); _eh.Add(b.TryStart); _eh.Add(b.TryEnd); _eh.Add(b.HandlerStart); _eh.Add(_ops.Count);
        _eh.Add(b.FilterStart);
        _ehTypes.Add(b.CatchType);
    }

    public override void BeginCatchBlock(Type? exceptionType)
    {
        var b = CurrentBlock();
        if (b.Kind == 1 && b.HandlerStart < 0)
        {
            if (exceptionType != null) throw new ArgumentException("A filter handler requires a null exception type.");
            Record(OpCodes.Endfilter, 0, null);
            b.HandlerStart = _ops.Count;
            return;
        }
        ArgumentNullException.ThrowIfNull(exceptionType);
        CloseHandler(b, 0);
        b.CatchType = exceptionType;
    }

    public override void BeginFinallyBlock()
    {
        var b = CurrentBlock();
        CloseHandler(b, 2);
        b.CatchType = null;
    }

    public override void EndExceptionBlock()
    {
        var b = CurrentBlock();
        if (b.Kind < 0 || (b.Kind == 1 && b.HandlerStart < 0))
            throw new InvalidOperationException("Exception block has no handler.");
        _blocks.Pop();
        CloseCurrentHandler(b);
        int end = _ops.Count;
        foreach (var site in b.LeaveSites) _imm[site] = end;
        MarkLabel(b.EndLabel);
    }

    public override void BeginExceptFilterBlock()
    {
        var b = CurrentBlock();
        CloseHandler(b, 1);
        b.FilterStart = _ops.Count;
        b.HandlerStart = -1;
        b.CatchType = null;
    }
    public override void BeginFaultBlock()
    {
        var b = CurrentBlock();
        CloseHandler(b, 4);
        b.CatchType = null;
    }
    public override void BeginScope() { }
    public override void EndScope() { }
    public override void UsingNamespace(string usingNamespace) { }

    [MethodImpl(MethodImplOptions.InternalCall)]
    private static extern System.Reflection.MethodInfo CompileRecorded(string name, Type returnType,
        Type[] parameterTypes, int[] ops, long[] imm, object?[] refs, Type[] locals, int[] localPins, int[] eh, Type?[] ehTypes,
        int maxStack);

    private int ComputeMaxStack(Type returnType)
    {
        var depths = new int[_ops.Count];
        System.Array.Fill(depths, -1);
        var pending = new Stack<int>();
        int maximum = 0;
        void Visit(int index, int depth)
        {
            if ((uint)index >= (uint)depths.Length) return;
            if (depths[index] == depth) return;
            if (depths[index] >= 0) throw new InvalidOperationException("Inconsistent evaluation stack depth.");
            depths[index] = depth;
            maximum = Math.Max(maximum, depth);
            pending.Push(index);
        }
        Visit(0, 0);
        for (int i = 0; i < _eh.Count; i += 6)
        {
            int kind = _eh[i];
            Visit(_eh[i + 3], kind == 0 || kind == 1 ? 1 : 0);
            if (kind == 1) Visit(_eh[i + 5], 1);
        }
        while (pending.Count != 0)
        {
            int i = pending.Pop();
            OpCode op = _codes[i];
            int pop = op.StackBehaviourPop switch
            {
                StackBehaviour.Pop0 => 0,
                StackBehaviour.Pop1 or StackBehaviour.Popi or StackBehaviour.Popref => 1,
                StackBehaviour.Pop1_pop1 or StackBehaviour.Popi_pop1 or StackBehaviour.Popi_popi or
                StackBehaviour.Popi_popi8 or StackBehaviour.Popi_popr4 or StackBehaviour.Popi_popr8 or
                StackBehaviour.Popref_pop1 or StackBehaviour.Popref_popi => 2,
                StackBehaviour.Popi_popi_popi or StackBehaviour.Popref_popi_pop1 or
                StackBehaviour.Popref_popi_popi or StackBehaviour.Popref_popi_popi8 or
                StackBehaviour.Popref_popi_popr4 or StackBehaviour.Popref_popi_popr8 or
                StackBehaviour.Popref_popi_popref => 3,
                StackBehaviour.Varpop when op.Value == OpCodes.Ret.Value => returnType == typeof(void) ? 0 : 1,
                StackBehaviour.Varpop when op.Value == OpCodes.Calli.Value && _refs[i] is Type[] signature =>
                    signature.Length + (((int)_imm[i] & (int)(CallingConventions.HasThis | CallingConventions.ExplicitThis)) ==
                        (int)CallingConventions.HasThis ? 1 : 0),
                StackBehaviour.Varpop when _refs[i] is System.Reflection.MethodBase method =>
                    method.GetParameters().Length + (op.Value != OpCodes.Newobj.Value && !method.IsStatic ? 1 : 0),
                StackBehaviour.Varpop when _refs[i] is object[] { Length: 2 } call =>
                    ((System.Reflection.MethodBase)call[0]).GetParameters().Length + ((Type[])call[1]).Length +
                    (op.Value != OpCodes.Newobj.Value && !((System.Reflection.MethodBase)call[0]).IsStatic ? 1 : 0),
                _ => throw new NotSupportedException("Cannot compute stack effect for " + op.Name),
            };
            int push = op.StackBehaviourPush switch
            {
                StackBehaviour.Push0 => 0,
                StackBehaviour.Push1 or StackBehaviour.Pushi or StackBehaviour.Pushi8 or
                StackBehaviour.Pushr4 or StackBehaviour.Pushr8 or StackBehaviour.Pushref => 1,
                StackBehaviour.Push1_push1 => 2,
                StackBehaviour.Varpush when op.Value == OpCodes.Newobj.Value => 1,
                StackBehaviour.Varpush when op.Value == OpCodes.Calli.Value && _refs[i] is Type[] signature =>
                    signature[0] == typeof(void) ? 0 : 1,
                StackBehaviour.Varpush when _refs[i] is System.Reflection.MethodInfo method =>
                    method.ReturnType == typeof(void) ? 0 : 1,
                StackBehaviour.Varpush when _refs[i] is object[] { Length: 2 } call =>
                    ((System.Reflection.MethodInfo)call[0]).ReturnType == typeof(void) ? 0 : 1,
                StackBehaviour.Varpush => 0,
                _ => throw new NotSupportedException("Cannot compute stack effect for " + op.Name),
            };
            int depth = depths[i] - pop + push;
            if (depth < 0) throw new InvalidOperationException("Evaluation stack underflow.");
            maximum = Math.Max(maximum, depth);
            if (op.Value == OpCodes.Leave.Value || op.Value == OpCodes.Leave_S.Value) depth = 0;
            if (op.Value != OpCodes.Jmp.Value &&
                (op.FlowControl == FlowControl.Branch || op.FlowControl == FlowControl.Cond_Branch))
            {
                if (op.Value == OpCodes.Switch.Value)
                {
                    foreach (int target in (int[])_refs[i]!) Visit(target, depth);
                }
                else Visit((int)_imm[i], depth);
            }
            if (op.FlowControl != FlowControl.Branch && op.FlowControl != FlowControl.Return &&
                op.FlowControl != FlowControl.Throw && op.Value != OpCodes.Endfinally.Value &&
                op.Value != OpCodes.Endfilter.Value && op.Value != OpCodes.Jmp.Value)
                Visit(i + 1, depth);
        }
        return maximum;
    }

    public System.Reflection.MethodInfo Compile(string name, Type returnType, Type[] parameterTypes)
    {
        if (_blocks.Count != 0)
            throw new NotSupportedException("Unclosed exception block.");
        int Target(int id)
        {
            if ((uint)id >= (uint)_labels.Count || _labels[id] < 0)
                throw new ArgumentException("Unmarked label.");
            return _labels[id];
        }
        foreach (var (site, label) in _branches)
        {
            int target = Target(label);
            if (_codes[site].OperandType == OperandType.ShortInlineBrTarget)
            {
                int targetOffset = target == _ops.Count ? _ilOffset : _offsets[target];
                int displacement = targetOffset - (_offsets[site] + _codes[site].Size + 1);
                if (displacement < sbyte.MinValue || displacement > sbyte.MaxValue)
                    throw new NotSupportedException("Short branch target is out of range.");
            }
            _imm[site] = target;
        }
        foreach (var (site, labels) in _switches)
        {
            var targets = new int[labels.Length];
            for (int i = 0; i < labels.Length; i++) targets[i] = Target(labels[i]);
            _refs[site] = targets;
        }
        return CompileRecorded(name, returnType, parameterTypes, _ops.ToArray(), _imm.ToArray(),
            _refs.ToArray(), _locals.ToArray(), _localPins.ToArray(), _eh.ToArray(), _ehTypes.ToArray(), ComputeMaxStack(returnType));
    }
}

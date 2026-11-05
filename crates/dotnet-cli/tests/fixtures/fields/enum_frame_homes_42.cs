using System;
using System.Runtime.CompilerServices;

enum SmallSigned : sbyte { Negative = -123 }
enum SmallUnsigned : byte { High = 250 }
enum ShortSigned : short { Negative = -30000 }
enum ShortUnsigned : ushort { High = 60000 }
enum EventId : int { First = 10000, Second = 10001 }
enum Unsigned : uint { High = 0xf1234567U }
struct ReferenceValue { public object Reference; public int Number; }
enum WideSigned : long { Negative = -9000000000L }
enum WideUnsigned : ulong { High = 0xf123456789abcdefUL }

public class Program {
    [MethodImpl(MethodImplOptions.NoInlining)]
    static string Name(EventId value) { return value.ToString(); }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static T Replace<T>(T value, T replacement) {
        ref T address = ref value;
        value = replacement;
        GC.Collect();
        return address;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static Type Identity<T>(ref T value) { return value.GetType(); }

    [MethodImpl(MethodImplOptions.NoInlining)]
    static T Default<T>() { return default(T); }

    public static int Main() {
        if (Name(EventId.First) != "First") return 1;
        EventId local = EventId.First;
        ref EventId saved = ref local;
        local = EventId.Second;
        if (saved != EventId.Second || Identity(ref local) != typeof(EventId)) return 2;
        saved = EventId.First;
        if (local != EventId.First) return 3;
        if (Replace(EventId.First, EventId.Second) != EventId.Second) return 4;
        if ((sbyte)Replace(default(SmallSigned), SmallSigned.Negative) != -123) return 5;
        if ((byte)Replace(default(SmallUnsigned), SmallUnsigned.High) != 250) return 6;
        if ((short)Replace(default(ShortSigned), ShortSigned.Negative) != -30000) return 7;
        if ((ushort)Replace(default(ShortUnsigned), ShortUnsigned.High) != 60000) return 8;
        if ((long)Replace(default(WideSigned), WideSigned.Negative) != -9000000000L) return 9;
        if ((ulong)Replace(default(WideUnsigned), WideUnsigned.High) != 0xf123456789abcdefUL) return 10;
        if (Replace(1, 42) != 42 || Replace(1.0, 2.5) != 2.5) return 11;
        if ((uint)Replace(Default<Unsigned>(), Unsigned.High) != 0xf1234567U) return 12;
        var reference = new object();
        var structure = Replace(Default<ReferenceValue>(), new ReferenceValue { Reference = reference, Number = 42 });
        if (!object.ReferenceEquals(structure.Reference, reference) || structure.Number != 42) return 13;
        return 42;
    }
}

using System;
using System.Reflection;

public interface IAlpha {
    int Alpha();
}

public interface IBeta {
    int Beta(int value);
}

public class ReflectionTarget : IAlpha, IBeta {
    public int FieldA;
    public int FieldB;
    private int _hidden;

    public int Value {
        get;
        set;
    }

    public static int SharedValue {
        get;
        set;
    }

    public ReflectionTarget() {
        FieldA = 7;
        FieldB = 13;
        _hidden = 29;
    }

    public ReflectionTarget(int seed) {
        FieldA = seed;
        FieldB = seed * 2;
        _hidden = seed * 3;
    }

    public int Alpha() => FieldA + _hidden;

    public int Beta(int value) => value + FieldB;

    public int Mix(int a, int b) => (a * 3) ^ (b + _hidden);

    private int Secret(int x) => (x << 1) ^ _hidden;

    public static int StaticMix(int x) => x ^ 0x55AA;

    public class Nested {
        public int Id;
    }
}

public class GenericTarget<T> where T : struct {
    public T Value;
    public T Echo(T value) => value;
}

public class Program {
    private const BindingFlags Flags =
        BindingFlags.Public
        | BindingFlags.NonPublic
        | BindingFlags.Instance
        | BindingFlags.Static
        | BindingFlags.DeclaredOnly;

    public static int Main() {
        Type target = typeof(ReflectionTarget);
        Type generic = typeof(GenericTarget<int>);
        int checksum = 0;

        for (int i = 0; i < 4_000; i++) {
            MethodInfo[] methods = target.GetMethods(Flags);
            ConstructorInfo[] constructors = target.GetConstructors(Flags);
            FieldInfo[] fields = target.GetFields(Flags);
            PropertyInfo[] properties = target.GetProperties(Flags);
            Type[] interfaces = target.GetInterfaces();

            if (methods.Length == 0) return 1;
            if (constructors.Length == 0) return 2;
            if (fields.Length == 0) return 3;
            if (interfaces.Length == 0) return 4;

            for (int j = 0; j < methods.Length; j++) {
                checksum += methods[j].Name.Length;
            }

            for (int j = 0; j < fields.Length; j++) {
                checksum ^= fields[j].Name.Length;
            }

            for (int j = 0; j < properties.Length; j++) {
                checksum += properties[j].Name.Length;
            }

            checksum += constructors.Length;
            checksum += interfaces.Length;

            checksum += generic.GetMethods(Flags).Length;
            checksum ^= generic.GetFields(Flags).Length;
            checksum += generic.GetGenericArguments().Length;
            checksum &= 0x7FFF_FFFF;
        }

        return checksum != 0 ? 0 : 7;
    }
}

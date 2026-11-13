using System;

ref struct CustomRefStruct { }
struct OrdinaryStruct { }
class OrdinaryClass { }

class Program {
    static void Show(Type type) => Console.WriteLine(type.IsByRefLike);

    static int Main() {
        Show(typeof(Span<int>));
        Show(typeof(ReadOnlySpan<int>));
        Show(typeof(Span<>));
        Show(typeof(CustomRefStruct));
        Show(typeof(OrdinaryStruct));
        Show(typeof(OrdinaryClass));
        Show(typeof(TypedReference));
        Show(typeof(int[]));
        Show(typeof(int).MakeByRefType());
        Show(typeof(Span<>).GetGenericArguments()[0]);
        return 42;
    }
}

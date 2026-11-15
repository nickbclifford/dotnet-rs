using System;
using System.Collections.Generic;

interface IStatic<T> { static abstract string Get<M>(); static abstract int Measure(T value); static virtual string Default<M>() => typeof(T).Name + "/" + typeof(M).Name; }
interface IInstance<T> { string Get<M>(); }
interface IDirect<T> { string Speak<M>(T value); }
struct Value<X> : IStatic<List<X>>, IInstance<List<X>> {
    public static string Get<M>() => typeof(X).Name + "/" + typeof(M).Name;
    public static int Measure(List<X> value) => value.Count;
    string IInstance<List<X>>.Get<M>() => typeof(X).Name + "/" + typeof(M).Name;
    public override string ToString() => typeof(X).Name;
}
struct Explicit<X> : IStatic<List<X>>, IStatic<string> {
    static string IStatic<List<X>>.Get<M>() => "List:" + typeof(X).Name + "/" + typeof(M).Name;
    static int IStatic<List<X>>.Measure(List<X> value) => value.Count;
    static string IStatic<string>.Get<M>() => "Text:" + typeof(X).Name + "/" + typeof(M).Name;
    static int IStatic<string>.Measure(string value) => value.Length;
}
struct Direct<X> : IDirect<List<X>> {
    public string Speak<M>(List<X> value) => typeof(X).Name + "/" + typeof(M).Name + "/" + value.Count;
}
struct Empty<X> { public X Item; }
class Parent<X> { public override string ToString() => typeof(X).Name; }
class Child<X> : Parent<List<X>> { }
class Reference<X> : IInstance<List<X>> {
    public string Get<M>() => typeof(X).Name + "/" + typeof(M).Name;
}
class Caller<A> {
    public static string Static<T, B>() where T : IStatic<List<int>> => T.Get<B>() + ";" + T.Default<B>() + ";" + T.Measure(new List<int> { 1, 2 });
    public static string StaticText<T, B>() where T : IStatic<string> => T.Get<B>() + ";" + T.Measure("hello");
    public static string Instance<T, B>(T value) where T : IInstance<List<int>> => value.Get<B>();
    public static string DirectCall<T, B>(T value) where T : IDirect<List<int>> => value.Speak<B>(new List<int> { 1, 2 });
    public static string Text<T, B>(T value) => value.ToString();
}
public class Program {
    public static int Main() {
        for (int i = 0; i < 2; i++) {
            string a = Caller<long>.Static<Value<int>, string>();
            string f = Caller<long>.Static<Explicit<int>, string>();
            string g = Caller<long>.StaticText<Explicit<int>, string>();
            string h = Caller<long>.DirectCall<Direct<int>, string>(new Direct<int>());
            string b = Caller<long>.Instance<Value<int>, string>(new Value<int>());
            string c = Caller<long>.Instance<Reference<int>, string>(new Reference<int>());
            string j = Caller<long>.Text<Child<int>, string>(new Child<int>());
            string d = Caller<long>.Text<Value<int>, string>(new Value<int>());
            string e = Caller<long>.Text<Empty<int>, string>(new Empty<int>());
            Console.WriteLine(a + ";" + f + ";" + g + ";" + h + ";" + j + ";" + b + ";" + c + ";" + d + ";" + e.StartsWith("Empty"));
            if (a != "Int32/String;List`1/String;2" || f != "List:Int32/String;List`1/String;2" || g != "Text:Int32/String;5" || j != "List`1" || h != "Int32/String/2" || b != "Int32/String" || c != b || d != "Int32" || !e.StartsWith("Empty")) return 1;
        }
        return 42;
    }
}

using System;
using System.Collections.Generic;
using System.Reflection;

// A callee's generic context comes from its own declaring type and MethodSpec, never from the
// generic method or type that happens to be calling it. typeof(...) is observable for every slot.
public class Program {
    interface IFoo<A> { string Describe(); }
    interface IPair<A, B> { string Pair(); }

    class Impl<U> : IFoo<List<U>> {
        public string Describe() { return "Impl<" + typeof(U).Name + ">"; }
    }
    class Swap<X, Y> : IPair<Y, X> {
        public string Pair() { return typeof(X).Name + "," + typeof(Y).Name; }
    }
    class Plain : IFoo<long> {
        public string Describe() { return "Plain"; }
    }
    abstract class Base { public abstract string G<M>(); }
    class Derived<U> : Base {
        public override string G<M>() { return typeof(U).Name + "/" + typeof(M).Name; }
    }
    class Statics<U> {
        public static string Name = typeof(U).Name;
        public static string Get() { return Name; }
    }
    class Finalized<U> {
        public static string Seen;
        ~Finalized() { Seen = typeof(U).Name; }
    }
    class Other<Q> {
        public string Who() { return typeof(Q).Name; }
        public static string StaticWho() { return typeof(Q).Name; }
    }
    class Holder {
        public static string Describe<M>() { return typeof(M).Name; }
    }
    class GenBase<X> {
        public virtual string Virt() { return "GenBase<" + typeof(X).Name + ">"; }
        public string NonVirt() { return "NonVirt<" + typeof(X).Name + ">"; }
        public object Boxed() { return new List<X>(); }
    }
    class FromGen : GenBase<string> { }
    class FromGenOverride : GenBase<string> {
        public override string Virt() { return "Override:" + base.Virt(); }
    }
    class Plain2 : IFoo<string> {
        public string Describe() { return "Plain2:" + typeof(IFoo<string>).GetGenericArguments()[0].Name; }
    }
    delegate string Del();

    static string Caller<T>(int i) {
        switch (i) {
            case 0: return ((IFoo<List<int>>)new Impl<int>()).Describe();
            case 1: return ((IPair<int, string>)new Swap<string, int>()).Pair();
            case 2: return ((IFoo<long>)new Plain()).Describe();
            case 3: return ((Base)new Derived<string>()).G<int>();
            case 4: return Statics<Guid>.Get();
            case 5: { Del d = new Impl<byte>().Describe; return d(); }
            case 6: { Del d = Holder.Describe<short>; return d(); }
            case 7: return new Other<char>().Who();
            case 8: return Other<double>.StaticWho();
            case 9: return typeof(Holder).GetMethod("Describe").MakeGenericMethod(typeof(float)).Invoke(null, null).ToString();
            case 10: return typeof(Other<int>).GetMethod("Who").Invoke(new Other<int>(), null).ToString();
            case 11: return typeof(Derived<byte>).GetMethod("G").MakeGenericMethod(typeof(char)).Invoke(new Derived<byte>(), null).ToString();
            case 12: { Func<string> f = () => typeof(T).Name; return f(); }
            case 13: return Describe<string>();
            case 14: return new FromGen().Virt();
            case 15: return new FromGen().NonVirt();
            case 16: return new FromGenOverride().Virt();
            case 17: return ((GenBase<string>)new FromGenOverride()).Virt();
            case 18: return ((IFoo<string>)new Plain2()).Describe();
            case 19: return new FromGen().Boxed().GetType().GetGenericArguments()[0].Name;
            case 20: { Del d = new FromGen().Virt; return d(); }
        }
        return "";
    }

    static string Describe<M>() { return typeof(M).Name; }

    static string Expected(int i) {
        switch (i) {
            case 0: return "Impl<Int32>";
            case 1: return "String,Int32";
            case 2: return "Plain";
            case 3: return "String/Int32";
            case 4: return "Guid";
            case 5: return "Impl<Byte>";
            case 6: return "Int16";
            case 7: return "Char";
            case 8: return "Double";
            case 9: return "Single";
            case 10: return "Int32";
            case 11: return "Byte/Char";
            case 12: return "Int64";
            case 13: return "String";
            case 14: return "GenBase<String>";
            case 15: return "NonVirt<String>";
            case 16: return "Override:GenBase<String>";
            case 17: return "Override:GenBase<String>";
            case 18: return "Plain2:String";
            case 19: return "String";
            case 20: return "GenBase<String>";
        }
        return "";
    }

    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    static void MakeFinalizable<T>() { new Finalized<T>(); }

    public static int Main() {
        for (int i = 0; i <= 20; i++) {
            string got = Caller<long>(i);
            if (got != Expected(i)) { Console.WriteLine(i + ": " + got); return 10 + i; }
        }
        MakeFinalizable<short>();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        if (Finalized<short>.Seen != "Int16") return 40;
        return 42;
    }
}

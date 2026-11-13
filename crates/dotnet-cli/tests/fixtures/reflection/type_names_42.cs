using System;
using System.Collections.Generic;

namespace Names {
    enum Color { Red }
    struct Plain { public int Value; }
    class Outer<T> { public class Inner<U> {} }
}
class Program {
    static void Show(Type t) { Console.WriteLine($"{t}|{t.FullName ?? "<null>"}"); }
    static int Main() {
        Show(typeof(int)); Show(typeof(Names.Color)); Show(typeof(Names.Plain));
        Show(typeof(Names.Outer<int>.Inner<string>)); Show(typeof(List<int>));
        Show(typeof(List<Names.Plain>)); Show(typeof(List<Names.Plain[]>)); Show(typeof(List<List<int>>));
        Show(typeof(List<>)); Show(typeof(int[])); Show(typeof(int[][])); Show(typeof(int[,]));
        Show(typeof(int).MakeByRefType());
        Show(typeof(List<>).GetGenericArguments()[0]);
        object p = new Names.Plain(); object a = new int[1][]; object g = new Names.Outer<int>.Inner<string>();
        Console.WriteLine(p.ToString()); Console.WriteLine(a.ToString()); Console.WriteLine(g.ToString());
        Console.WriteLine($"{p.GetType()}|{a.GetType()}|{g.GetType()}");
        return 42;
    }
}

using System;
using System.Reflection;

public enum Color { Red = 1, Green = 2 }
public class Animal { }
public class Dog : Animal { }
public static class Target
{
    public static long Long(long v) => v;
    public static int Int(int v) => v;
    public static short Short(short v) => v;
    public static double Double(double v) => v;
    public static string Str(string v) => v;
    public static int Obj(object v) => v == null ? 0 : 1;
    public static int Col(Color c) => (int)c;
    public static string Pet(Animal a) => a.GetType().Name;
    public static string DogOnly(Dog a) => a.GetType().Name;
    public static int Ints(int a, int b) => a * 10 + b;
    public static bool Bool(bool b) => b;
    public static char Chr(char c) => c;
}
public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }
    static bool Throws<T>(Action action) where T : Exception
    {
        try { action(); return false; }
        catch (T) { return true; }
        catch (Exception) { return false; }
    }
    static MethodInfo M(string name) => typeof(Target).GetMethod(name)!;
    public static int Main()
    {
        Check(1, Throws<ArgumentException>(() => M("Int").Invoke(null, new object[] { "x" })));
        Check(2, Throws<ArgumentException>(() => M("Str").Invoke(null, new object[] { 1 })));
        Check(3, (long)M("Long").Invoke(null, new object[] { 5 })! == 5);
        Check(4, (long)M("Long").Invoke(null, new object[] { (short)6 })! == 6);
        Check(5, (long)M("Long").Invoke(null, new object[] { (byte)7 })! == 7);
        Check(6, Throws<ArgumentException>(() => M("Short").Invoke(null, new object[] { 5 })));
        Check(7, Throws<ArgumentException>(() => M("Int").Invoke(null, new object[] { 5L })));
        Check(8, (double)M("Double").Invoke(null, new object[] { 3 })! == 3.0);
        Check(9, (double)M("Double").Invoke(null, new object[] { 1.5f })! == 1.5);
        Check(10, Throws<ArgumentException>(() => M("Int").Invoke(null, new object[] { 1.5 })));
        Check(11, (int)M("Col").Invoke(null, new object[] { Color.Green })! == 2);
        Check(12, (int)M("Col").Invoke(null, new object[] { 1 })! == 1);
        Check(13, (int)M("Int").Invoke(null, new object[] { Color.Green })! == 2);
        Check(14, Throws<ArgumentException>(() => M("Col").Invoke(null, new object[] { "x" })));
        Check(15, (int)M("Obj").Invoke(null, new object[] { 1 })! == 1);
        Check(16, (int)M("Obj").Invoke(null, new object?[] { null })! == 0);
        Check(17, (string)M("Pet").Invoke(null, new object[] { new Dog() })! == "Dog");
        Check(18, Throws<ArgumentException>(() => M("DogOnly").Invoke(null, new object[] { new Animal() })));
        Check(19, Throws<ArgumentException>(() => M("DogOnly").Invoke(null, new object[] { "x" })));
        Check(20, (int)M("Int").Invoke(null, new object?[] { null })! == 0);
        Check(21, M("Str").Invoke(null, new object?[] { null }) == null);
        Check(22, (int)M("Ints").Invoke(null, new object[] { 4, 2 })! == 42);
        Check(23, Throws<ArgumentException>(() => M("Ints").Invoke(null, new object[] { 4, "2" })));
        Check(24, Throws<ArgumentException>(() => M("Bool").Invoke(null, new object[] { 1 })));
        Check(25, (int)M("Int").Invoke(null, new object[] { 'a' })! == 97);
        Check(26, Throws<ArgumentException>(() => M("Chr").Invoke(null, new object[] { 97 })));
        return failures == 0 ? 42 : 1;
    }
}

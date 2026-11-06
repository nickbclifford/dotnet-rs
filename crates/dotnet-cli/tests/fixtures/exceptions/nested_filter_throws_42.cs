using System;
public class Program {
 static void Thrower() { throw new Exception("original"); }
 static bool Inner() { Console.WriteLine("inner"); throw new Exception("filter"); }
 static bool Outer() {
  try { Thrower(); }
  catch (Exception) when (Inner()) { Console.WriteLine("inner caught"); }
  return true;
 }
 public static int Main() {
  try { Thrower(); }
  catch (Exception) when (Outer()) { Console.WriteLine("outer caught"); return 42; }
  catch (Exception) { Console.WriteLine("outer fallback"); return 42; }
  return 1;
 }
}

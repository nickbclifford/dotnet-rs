using System;
public class Program {
 static void Thrower() { throw new Exception("original"); }
 static bool Inner() { Console.WriteLine("inner"); return true; }
 static bool Outer() {
  try { Thrower(); }
  catch (Exception) when (Inner()) { Console.WriteLine("inner caught"); }
  catch (Exception) { Console.WriteLine("fallback"); }
  return true;
 }
 public static int Main() {
  try { Thrower(); }
  catch (Exception) when (Outer()) { Console.WriteLine("outer caught"); return 42; }
  return 1;
 }
}

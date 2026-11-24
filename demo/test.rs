fn main() {
    let seconds = String::from("1640970000");
    let secondsLen = calculate_length(&seconds);  // 借用
    println!("The length of '{}' is {}.", seconds, secondsLen);

    let milliseconds = String::from("1671160204198");
    let millisecondsLen = calculate_length(&milliseconds);  // 借用
    println!("The length of '{}' is {}.", milliseconds, millisecondsLen);

    let microseconds = String::from("1671160204197999910");
    let microsecondsLen = calculate_length(&microseconds);  // 借用
    println!("The length of '{}' is {}.", microseconds, microsecondsLen);

    let nanoseconds = String::from("1671160204197999910");
    let nanosecondsLen = calculate_length(&nanoseconds);  // 借用
    println!("The length of '{}' is {}.", nanoseconds, nanosecondsLen);
}

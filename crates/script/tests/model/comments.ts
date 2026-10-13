function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  eq([workbook.getComments().length, sheet.getComments().length, sheet.getNotes().length, workbook.getNotes().length], [0, 0, 0, 0]);
  const before = Date.now();

  // A thread, its replies, its state.
  const comment = workbook.addComment("B2", "Is ten enough?");
  eq([comment.getContent(), comment.getAuthorName(), comment.getAuthorEmail(), comment.getContentType()], ["Is ten enough?", "Ada Script", "", "Plain"]);
  eq([comment.getLocation().getAddress(), comment.getResolved(), comment.getReplies().length], ["Sheet1!B2", false, 0]);
  const written = comment.getCreationDate();
  eq(written instanceof Date && Math.abs(written.getTime() - before) < 120000, true);
  eq(workbook.getComment(comment.getId())?.getContent(), "Is ten enough?");
  eq(sheet.getComment(comment.getId())?.getId(), comment.getId());
  eq([workbook.getComment("nope"), sheet.getComment("c9-0-0")], [undefined, undefined]);
  eq(workbook.getCommentByCell("B2").getId(), comment.getId());
  eq(sheet.getCommentByCell(sheet.getRange("B2")).getId(), comment.getId());
  throws(() => sheet.getCommentByCell("A1"), "doesn't exist");
  throws(() => workbook.addComment("Sheet1!B2", "again"), "already has");
  comment.setContent("Is 10 enough?");
  eq([comment.getContent(), comment.getRichContent(), comment.getMentions()], ["Is 10 enough?", "Is 10 enough?", []]);

  const first = comment.addCommentReply("Yes");
  const second = comment.addCommentReply("No, order more", ExcelScript.ContentType.plain);
  eq(comment.getReplies().map(r => r.getContent()), ["Yes", "No, order more"]);
  eq([second.getAuthorName(), second.getAuthorEmail(), second.getContentType(), second.getResolved()], ["Ada Script", "", "Plain", false]);
  eq([second.getLocation().getAddress(), second.getParentComment().getId(), second.getCreationDate() instanceof Date], ["Sheet1!B2", comment.getId(), true]);
  eq(comment.getCommentReply(second.getId())?.getContent(), "No, order more");
  eq(comment.getCommentReply("c0-1-1-7"), undefined);
  eq(workbook.getCommentByReplyId(second.getId()).getId(), comment.getId());
  eq(sheet.getCommentByReplyId(first.getId()).getId(), comment.getId());
  throws(() => workbook.getCommentByReplyId(comment.getId()), "doesn't exist");
  second.setContent("No: order more");
  eq([second.getContent(), second.getRichContent(), second.getMentions().length], ["No: order more", "No: order more", 0]);

  // Mentions.
  const people = [{ email: "ann@example.org", id: 0, name: "Ann Lee" }, { email: "bob@example.org", id: 1, name: "Bob" }];
  second.updateMentions({ richContent: "<at id=\"1\">Bob</at> and <at id=\"0\">Ann Lee</at>, please check", mentions: people });
  eq(second.getContent(), "@Bob and @Ann Lee, please check");
  eq(second.getRichContent(), "<at id=\"0\">Bob</at> and <at id=\"1\">Ann Lee</at>, please check");
  eq([second.getContentType(), second.getMentions()], ["Mention", [{ email: "bob@example.org", id: 0, name: "Bob" }, { email: "ann@example.org", id: 1, name: "Ann Lee" }]]);
  comment.updateMentions({ richContent: "<at id=\"0\">Ann Lee</at>: is 10 enough?", mentions: people });
  eq([comment.getContent(), comment.getContentType(), comment.getMentions().length], ["@Ann Lee: is 10 enough?", "Mention", 1]);
  throws(() => comment.updateMentions({ richContent: "<at id=\"5\">Nobody</at>", mentions: people }), "invalid");
  const named = sheet.addComment("C3", { richContent: "Ask <at id=\"0\">Ann Lee</at>", mentions: people }, ExcelScript.ContentType.mention);
  eq([named.getContent(), named.getMentions()[0].email], ["Ask @Ann Lee", "ann@example.org"]);

  // A resolved thread takes no change until it is reopened.
  comment.setResolved(true);
  eq([comment.getResolved(), first.getResolved()], [true, true]);
  throws(() => comment.addCommentReply("late"), "resolved");
  throws(() => first.setContent("x"), "resolved");
  throws(() => first.delete(), "resolved");
  comment.setResolved(false);
  first.delete();
  eq(comment.getReplies().map(r => r.getContent()), ["@Bob and @Ann Lee, please check"]);
  eq([workbook.getComments().length, sheet.getComments().length], [2, 2]);
  named.delete();
  throws(() => named.getContent(), "doesn't exist");

  // Notes.
  const note = sheet.addNote("A4", "Reorder at 3");
  eq([note.getContent(), note.getAuthorName(), note.getLocation().getAddress(), note.getVisible()], ["Reorder at 3", "Ada Script", "Sheet1!A4", false]);
  throws(() => sheet.addNote("B2", "on a comment"), "already has");
  throws(() => workbook.addComment("A4", "on a note"), "already has");
  note.setContent("Reorder at 5");
  note.setVisible(true);
  eq([note.getContent(), note.getVisible()], ["Reorder at 5", true]);
  // The box Excel gives a new note: 144 by 79 pixels.
  near(note.getWidth(), 108);
  near(note.getHeight(), 59.25);
  note.setWidth(150);
  note.setHeight(60);
  near(note.getWidth(), 150);
  near(note.getHeight(), 60);
  throws(() => note.setWidth(-1), "invalid");
  const other = workbook.addNote("Sheet1!C1", "Unit price");
  eq([sheet.getNotes().length, workbook.getNotes().length], [2, 2]);
  eq([sheet.getNote("A4")?.getContent(), sheet.getNote(0)?.getContent(), sheet.getNote(1)?.getContent(), sheet.getNote(2), sheet.getNote("B9")], ["Reorder at 5", "Unit price", "Reorder at 5", undefined, undefined]);
  other.delete();
  throws(() => other.getContent(), "doesn't exist");
  eq(sheet.getComments().length, 1);
}

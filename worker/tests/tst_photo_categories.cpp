#include <QTest>

#include "photo_categories.h"

using namespace protondrive;

// Covers issue #90's first target: photo_categories.h's 9-entry slug table
// (see its own doc comment — must stay in sync with core/src/photos.rs's
// PhotoCategory::slug(), with no compile-time link between the two, so a
// typo here or there is exactly the kind of regression a test catches that
// a live Dolphin click-through won't).
class TestPhotoCategories : public QObject
{
    Q_OBJECT

private Q_SLOTS:
    void slugsHasNineEntries()
    {
        QCOMPARE(photoCategorySlugs().size(), 9);
    }

    void slugsHasNoDuplicates()
    {
        const QStringList &slugs = photoCategorySlugs();
        QCOMPARE(QSet<QString>(slugs.begin(), slugs.end()).size(), slugs.size());
    }

    void everySlugHasALabelAndAnIcon()
    {
        for (const QString &slug : photoCategorySlugs()) {
            QVERIFY2(!photoCategoryLabel(slug).isEmpty(), qPrintable(slug));
            QVERIFY2(!photoCategoryIcon(slug).isEmpty(), qPrintable(slug));
        }
    }

    void unknownSlugReturnsEmpty()
    {
        QVERIFY(photoCategoryLabel(QStringLiteral("not-a-real-category")).isEmpty());
        QVERIFY(photoCategoryIcon(QStringLiteral("not-a-real-category")).isEmpty());
    }
};

QTEST_GUILESS_MAIN(TestPhotoCategories)
#include "tst_photo_categories.moc"
